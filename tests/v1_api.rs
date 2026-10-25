//! End-to-end checks for the required v1 HTTP API.
//!
//! This test owns all client rows in `V1_API_TEST_DATABASE_URL`. Point it at a
//! fresh, disposable PostgreSQL database; it refuses to run against a database
//! that already contains client data.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use aeza_intelligence::{
    Dataset, PgAdapter,
    market::Offer,
    market_store::{ObservationSource, save_offers},
    model::{
        Account, BonusState, PaymentTerm, Service, ServiceStatus, Transaction, TransactionStatus,
        TransactionType,
    },
};
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};

const LOCAL_USER: &str = "v1-api-test@aeza.test";
const LOCAL_PASSWORD: &str = "isolated-api-test-password-2026";
const TEST_WS_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

struct TestWebSocket(BufReader<TcpStream>);

impl TestWebSocket {
    fn connect(address: &str, cookie: &str) -> Self {
        let stream = TcpStream::connect(address).expect("connect to isolated API WebSocket");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = BufReader::new(stream);
        write!(
            socket.get_mut(),
            "GET /api/client/ws HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {TEST_WS_KEY}\r\nSec-WebSocket-Version: 13\r\nOrigin: http://{address}\r\nCookie: {cookie}\r\n\r\n"
        )
        .expect("write WebSocket upgrade request");
        socket.get_mut().flush().unwrap();

        let mut status = String::new();
        socket.read_line(&mut status).unwrap();
        assert!(
            status.contains(" 101 "),
            "authenticated WebSocket upgrade failed: {status}"
        );
        loop {
            let mut header = String::new();
            socket.read_line(&mut header).unwrap();
            if header == "\r\n" || header.is_empty() {
                break;
            }
        }
        Self(socket)
    }

    fn read_frame(&mut self) -> std::io::Result<(u8, Vec<u8>)> {
        let mut first = [0_u8; 2];
        self.0.read_exact(&mut first)?;
        let opcode = first[0] & 0x0f;
        let masked = first[1] & 0x80 != 0;
        let mut length = u64::from(first[1] & 0x7f);
        match length {
            126 => {
                let mut extended = [0_u8; 2];
                self.0.read_exact(&mut extended)?;
                length = u64::from(u16::from_be_bytes(extended));
            }
            127 => {
                let mut extended = [0_u8; 8];
                self.0.read_exact(&mut extended)?;
                length = u64::from_be_bytes(extended);
            }
            _ => {}
        }
        assert!(
            length <= 16 * 1024 * 1024,
            "WebSocket frame is unexpectedly large"
        );
        let mut mask = [0_u8; 4];
        if masked {
            self.0.read_exact(&mut mask)?;
        }
        let mut payload = vec![0; length as usize];
        self.0.read_exact(&mut payload)?;
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % mask.len()];
            }
        }
        Ok((opcode, payload))
    }

    fn recv_json(&mut self) -> Value {
        loop {
            let (opcode, payload) = self.read_frame().expect("read WebSocket event");
            match opcode {
                0x1 => return serde_json::from_slice(&payload).expect("WebSocket JSON message"),
                0x8 => panic!("WebSocket closed before the expected event"),
                0x9 => self.send_control(0xA, &payload),
                0xA => {}
                _ => panic!("unexpected WebSocket frame opcode {opcode}"),
            }
        }
    }

    fn assert_no_message_for(&mut self, duration: Duration) {
        self.0.get_ref().set_read_timeout(Some(duration)).unwrap();
        let result = self.read_frame();
        self.0
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        assert!(
            result.is_err_and(|error| matches!(
                error.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            )),
            "duplicate WebSocket event received after dataset and score updates"
        );
    }

    fn send_control(&mut self, opcode: u8, payload: &[u8]) {
        assert!(
            payload.len() < 126,
            "unexpectedly large WebSocket control frame"
        );
        let mask = [0xA1, 0xB2, 0xC3, 0xD4];
        let mut frame = vec![0x80 | opcode, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % 4]),
        );
        self.0.get_mut().write_all(&frame).unwrap();
        self.0.get_mut().flush().unwrap();
    }
}

struct ApiServer(Child);

impl ApiServer {
    fn start(database_url: &str, address: &str) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_market_api"))
            .args(["--listen", address])
            .env("DATABASE_URL", database_url)
            .env("LOCAL_ADMIN_USER", LOCAL_USER)
            .env("LOCAL_ADMIN_PASSWORD", LOCAL_PASSWORD)
            .env("PUBLIC_URL", format!("http://{address}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start isolated market API");
        Self(child)
    }

    fn assert_running(&mut self) {
        assert!(
            self.0.try_wait().unwrap().is_none(),
            "market API exited during test"
        );
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture_for(ids: &[i64]) -> Dataset {
    let now = Utc::now();
    let ids = ids.to_vec();
    let accounts = ids
        .iter()
        .map(|id| Account {
            id: *id,
            email: format!("customer-{id}@integration.test"),
            balance: 10_000 + id,
            withdraw_balance: 8_000 + id,
            total_replenished: 20_000 + id,
            bonus_balance: 1_000,
            bonus_state: BonusState::Unlocked,
        })
        .collect();
    let mut services: Vec<_> = ids
        .iter()
        .map(|id| Service {
            id: *id,
            name: format!("vps-{id}"),
            owner_id: *id,
            payment_term: PaymentTerm::Month,
            auto_prolong: true,
            created_at: now - ChronoDuration::days(20),
            expires_at: now + ChronoDuration::days(10),
            status: ServiceStatus::Active,
            price: 500,
        })
        .collect();
    let mut transactions: Vec<_> = ids
        .iter()
        .map(|id| Transaction {
            id: id * 10,
            amount: 500,
            status: TransactionStatus::Performed,
            performed_at: Some(now - ChronoDuration::days(20)),
            created_at: now - ChronoDuration::days(20),
            kind: TransactionType::Order,
            service_id: *id,
        })
        .collect();

    // One account carries every source service and transaction state so the
    // HTTP contract test verifies preservation instead of normalizing values.
    if ids.contains(&2) {
        for (offset, status) in [
            ServiceStatus::Suspended,
            ServiceStatus::ProlongWait,
            ServiceStatus::Deleted,
            ServiceStatus::Blocked,
        ]
        .into_iter()
        .enumerate()
        {
            services.push(Service {
                id: 2_000 + offset as i64,
                name: format!("status-{status:?}"),
                owner_id: 2,
                payment_term: PaymentTerm::Month,
                auto_prolong: false,
                created_at: now - ChronoDuration::days(20),
                expires_at: now + ChronoDuration::days(10),
                status,
                price: 100,
            });
        }
        transactions.extend([
            Transaction {
                id: 20_001,
                amount: 100,
                status: TransactionStatus::Performed,
                performed_at: Some(now - ChronoDuration::days(20)),
                created_at: now - ChronoDuration::days(20),
                kind: TransactionType::Prolong,
                service_id: 2,
            },
            Transaction {
                id: 20_002,
                amount: 100,
                status: TransactionStatus::NotPerformed,
                performed_at: None,
                created_at: now - ChronoDuration::days(1),
                kind: TransactionType::Order,
                service_id: 2_000,
            },
        ]);
    }
    Dataset {
        accounts,
        services,
        transactions,
    }
}

async fn ready(client: &Client, base: &str, server: &mut ApiServer) {
    for _ in 0..100 {
        server.assert_running();
        if client
            .get(format!("{base}/api/v1/openapi.json"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("isolated market API did not become ready at {base}");
}

async fn login(client: &Client, base: &str) -> String {
    let response = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"username": LOCAL_USER, "password": LOCAL_PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let set_cookie = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(
        !set_cookie.contains("; Secure"),
        "HTTP test origin must not set Secure"
    );
    set_cookie.split(';').next().unwrap().to_owned()
}

async fn get(client: &Client, base: &str, path: &str, cookie: &str) -> Response {
    client
        .get(format!("{base}{path}"))
        .header(reqwest::header::COOKIE, cookie)
        .send()
        .await
        .unwrap()
}

async fn patch_json(client: &Client, url: String, cookie: Option<&str>, value: &Value) -> Response {
    let mut request = client.patch(url).json(value);
    if let Some(cookie) = cookie {
        request = request.header(reqwest::header::COOKIE, cookie);
    }
    request.send().await.unwrap()
}

async fn put_json(client: &Client, url: String, cookie: &str, value: &Value) -> Response {
    client
        .put(url)
        .header(reqwest::header::COOKIE, cookie)
        .json(value)
        .send()
        .await
        .unwrap()
}

async fn post_json(client: &Client, url: String, cookie: &str, value: &Value) -> Response {
    client
        .post(url)
        .header(reqwest::header::COOKIE, cookie)
        .json(value)
        .send()
        .await
        .unwrap()
}

async fn seed_database_session(pool: &PgPool, email: &str, role: &str, token: &str) -> String {
    let admin_id: i64 = sqlx::query_scalar(
        "INSERT INTO public.admins (email, name, role) VALUES ($1, $1, $2) \
         ON CONFLICT (lower(email)) DO UPDATE SET role = excluded.role, \"isActive\" = true \
         RETURNING id",
    )
    .bind(email)
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.admin_sessions (token_hash, admin_id, expires_at) \
         VALUES (sha256(convert_to($1, 'UTF8')), $2, now() + interval '1 hour') \
         ON CONFLICT (token_hash) DO UPDATE SET admin_id = excluded.admin_id, expires_at = excluded.expires_at",
    )
    .bind(token)
    .bind(admin_id)
    .execute(pool)
    .await
    .unwrap();
    format!("aeza_session={token}")
}

#[tokio::test]
async fn required_v1_api_auth_customer_settings_snapshot_and_docs_contract() {
    let Some(database_url) = std::env::var_os("V1_API_TEST_DATABASE_URL") else {
        eprintln!("v1 API end-to-end test skipped: V1_API_TEST_DATABASE_URL is unset");
        return;
    };
    let database_url = database_url.to_str().expect("database URL is UTF-8");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await
        .expect("connect to isolated test database");
    let adapter = PgAdapter::from_pool(pool.clone());
    adapter.migrate().await.unwrap();
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM public.accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        existing, 0,
        "V1_API_TEST_DATABASE_URL must be empty; refusing to replace existing data"
    );
    adapter
        .replace(&fixture_for(&[2, 10, 11, 100]))
        .await
        .unwrap();
    pool.close().await;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    let base = format!("http://{address}");
    let client = Client::new();
    let mut server = ApiServer::start(database_url, &address);
    ready(&client, &base, &mut server).await;

    for path in [
        "/api/v1/auth/me",
        "/api/v1/customers",
        "/api/v1/customers/10",
        "/api/v1/analytics/summary",
        "/api/v1/model/settings",
    ] {
        assert_eq!(
            get(&client, &base, path, "").await.status(),
            StatusCode::UNAUTHORIZED,
            "protected GET {path} rejects an anonymous session"
        );
    }
    let rejected_login = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"username": LOCAL_USER, "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected_login.status(), StatusCode::UNAUTHORIZED);
    assert!(
        rejected_login
            .text()
            .await
            .unwrap()
            .find(LOCAL_PASSWORD)
            .is_none()
    );
    let malformed_login = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&json!({"username": LOCAL_USER}))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed_login.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        client
            .post(format!("{base}/api/v1/auth/logout"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{base}/api/v1/docs"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
        "documentation is public by contract"
    );
    assert_eq!(
        client
            .get(format!("{base}/metrics"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
        "metrics are public by contract"
    );
    let cross_origin_login = client
        .post(format!("{base}/api/v1/auth/login"))
        .header(reqwest::header::ORIGIN, "http://attacker.invalid")
        .json(&json!({"username": LOCAL_USER, "password": LOCAL_PASSWORD}))
        .send()
        .await
        .unwrap();
    assert_eq!(cross_origin_login.status(), StatusCode::FORBIDDEN);

    let cookie = login(&client, &base).await;
    let me: Value = get(&client, &base, "/api/v1/auth/me", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["email"], LOCAL_USER);
    assert_eq!(me["role"], "superadmin");

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .unwrap();
    let viewer_cookie = seed_database_session(
        &pool,
        "v1-api-viewer@aeza.test",
        "viewer",
        "isolated-viewer-session-token",
    )
    .await;
    let expired_cookie = seed_database_session(
        &pool,
        "v1-api-expired@aeza.test",
        "viewer",
        "isolated-expired-session-token",
    )
    .await;
    sqlx::query(
        "UPDATE public.admin_sessions SET expires_at = now() - interval '1 second' \
         WHERE token_hash = sha256(convert_to($1, 'UTF8'))",
    )
    .bind("isolated-expired-session-token")
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        get(&client, &base, "/api/v1/auth/me", &expired_cookie)
            .await
            .status(),
        StatusCode::UNAUTHORIZED,
        "expired persisted sessions are refused"
    );
    assert_eq!(
        get(
            &client,
            &base,
            "/api/v1/auth/me",
            "aeza_session=unknown-token"
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED,
        "unknown session tokens are refused"
    );
    assert_eq!(
        get(&client, &base, "/api/v1/customers?sort=id", &viewer_cookie)
            .await
            .status(),
        StatusCode::OK,
        "viewer can read customer data"
    );
    assert_eq!(
        patch_json(
            &client,
            format!("{base}/api/v1/model/settings"),
            Some(&viewer_cookie),
            &json!({"weights": {"paymentCoverage": 35, "noAutoProlong": 15, "prolongWait": 15, "failedProlong": 10, "expiringRevenue": 25}, "mediumThreshold": 35, "highThreshold": 65, "renewalWindowDays": 14, "historicalMinDays": 30}),
        )
        .await
        .status(),
        StatusCode::FORBIDDEN,
        "viewer cannot edit model settings"
    );
    assert_eq!(
        put_json(
            &client,
            format!("{base}/api/v1/source/snapshot"),
            &viewer_cookie,
            &json!({"accounts": [], "services": [], "transactions": []}),
        )
        .await
        .status(),
        StatusCode::FORBIDDEN,
        "viewer cannot replace source data"
    );

    let asc: Value = get(
        &client,
        &base,
        "/api/v1/customers?sort=id&direction=asc&limit=10",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    let ids: Vec<i64> = asc["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["account"]["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [2, 10, 11, 100], "ID sorting is numeric");
    assert_eq!(asc["total"], 4);

    let desc: Value = get(
        &client,
        &base,
        "/api/v1/customers?sort=id&direction=desc&limit=10",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    let ids: Vec<i64> = desc["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["account"]["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [100, 11, 10, 2]);

    let page: Value = get(
        &client,
        &base,
        "/api/v1/customers?sort=id&page=2&limit=2",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(page["items"][0]["account"]["id"], 11);
    assert_eq!(page["items"][1]["account"]["id"], 100);
    assert_eq!(page["page"], 2);
    assert_eq!(page["limit"], 2);

    for (sort, ascending, descending) in [
        ("email", [100, 10, 11, 2], [2, 11, 10, 100]),
        ("balance", [2, 10, 11, 100], [100, 11, 10, 2]),
        ("totalReplenished", [2, 10, 11, 100], [100, 11, 10, 2]),
    ] {
        for (direction, expected) in [("asc", ascending), ("desc", descending)] {
            let response: Value = get(
                &client,
                &base,
                &format!("/api/v1/customers?sort={sort}&direction={direction}&limit=10"),
                &cookie,
            )
            .await
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
            let actual: Vec<i64> = response["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["account"]["id"].as_i64().unwrap())
                .collect();
            assert_eq!(actual, expected, "{sort} {direction}");
        }
    }

    for direction in ["asc", "desc"] {
        let sorted: Value = get(
            &client,
            &base,
            &format!("/api/v1/customers?sort=score&direction={direction}&limit=10"),
            &cookie,
        )
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        let values: Vec<(i64, i64)> = sorted["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["risk"]["score"].as_i64().unwrap(),
                    item["account"]["id"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(values.len(), 4);
        for pair in values.windows(2) {
            let correctly_ordered = if direction == "asc" {
                pair[0].0 < pair[1].0 || (pair[0].0 == pair[1].0 && pair[0].1 <= pair[1].1)
            } else {
                pair[0].0 > pair[1].0 || (pair[0].0 == pair[1].0 && pair[0].1 <= pair[1].1)
            };
            assert!(correctly_ordered, "score sort {direction}: {values:?}");
        }
    }

    let search: Value = get(
        &client,
        &base,
        "/api/v1/customers?search=customer-10%40integration.test&sort=id",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(search["total"], 1);
    assert_eq!(search["items"][0]["account"]["id"], 10);
    let empty_search: Value = get(
        &client,
        &base,
        "/api/v1/customers?search=missing-customer&sort=id",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(empty_search["total"], 0);
    let injection_search: Value = get(
        &client,
        &base,
        "/api/v1/customers?search=%27%20OR%201%3D1--&sort=id",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(
        injection_search["total"], 0,
        "search values are SQL parameters"
    );
    let all_risks: Value = get(
        &client,
        &base,
        "/api/v1/customers?risk=all&sort=id",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(all_risks["total"], 4);
    for level in ["low", "medium", "high", "limited"] {
        let filtered: Value = get(
            &client,
            &base,
            &format!("/api/v1/customers?risk={level}&sort=id"),
            &cookie,
        )
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        assert!(
            filtered["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| { item["risk"]["level"] == level })
        );
        assert_eq!(
            filtered["total"].as_u64().unwrap() as usize,
            filtered["items"].as_array().unwrap().len()
        );
    }

    for invalid_query in [
        "/api/v1/customers?sort=unknown",
        "/api/v1/customers?direction=sideways",
        "/api/v1/customers?page=0",
        "/api/v1/customers?limit=501",
        "/api/v1/customers?risk=unknown",
    ] {
        assert_eq!(
            get(&client, &base, invalid_query, &cookie).await.status(),
            StatusCode::BAD_REQUEST,
            "invalid query should be rejected: {invalid_query}"
        );
    }

    let detail: Value = get(&client, &base, "/api/v1/customers/10", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["account"]["id"], 10);
    assert_eq!(detail["services"][0]["status"], "active");
    assert_eq!(detail["transactions"][0]["status"], "performed");
    assert_eq!(detail["transactions"][0]["type"], "order");
    let status_detail: Value = get(&client, &base, "/api/v1/customers/2", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let service_states: Vec<&str> = status_detail["services"]
        .as_array()
        .unwrap()
        .iter()
        .map(|service| service["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        service_states,
        ["active", "suspended", "prolong_wait", "deleted", "blocked"]
    );
    let transaction_states: Vec<(&str, &str)> = status_detail["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|transaction| {
            (
                transaction["status"].as_str().unwrap(),
                transaction["type"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        transaction_states,
        [
            ("performed", "order"),
            ("performed", "prolong"),
            ("not_performed", "order")
        ]
    );
    let customer_json = serde_json::to_string(&status_detail)
        .unwrap()
        .to_lowercase();
    assert!(!customer_json.contains("password"));
    assert!(!customer_json.contains("token_hash"));
    assert!(!customer_json.contains("client_secret"));
    assert_eq!(
        get(&client, &base, "/api/v1/customers/404", &cookie)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&client, &base, "/api/v1/customers/not-an-id", &cookie)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let cashflow: Value = get(&client, &base, "/api/client/cashflow/10", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cashflow["includedServices"], 1);
    assert_eq!(cashflow["unresolvedServices"], 0);
    assert_eq!(
        cashflow["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|window| window["days"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [7, 30, 60, 90]
    );
    let projected_outflows: Vec<i64> = cashflow["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| window["projectedOutflowCents"].as_i64().unwrap())
        .collect();
    assert_eq!(projected_outflows, [0, 500, 1_000, 1_500]);
    assert!(
        cashflow["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|window| window["deficitCents"] == 0)
    );

    let snapshot_settings = json!({
        "weights": {
            "paymentCoverage": 35,
            "noAutoProlong": 15,
            "prolongWait": 15,
            "failedProlong": 10,
            "expiringRevenue": 25,
            "priceRisk": 0
        },
        "mediumThreshold": 35,
        "highThreshold": 65,
        "renewalWindowDays": 14,
        "historicalMinDays": 30
    });
    let snapshot: Value = post_json(
        &client,
        format!("{base}/api/client/snapshot"),
        &cookie,
        &snapshot_settings,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(snapshot.as_array().unwrap().len(), 4);
    for assessment in snapshot.as_array().unwrap() {
        assert!((0..=100).contains(&assessment["score"].as_i64().unwrap()));
        assert_eq!(assessment["assessmentStatus"], "sufficient_data");
        assert_eq!(assessment["settingsVersion"], 0);
    }
    let repeated_snapshot: Value = post_json(
        &client,
        format!("{base}/api/client/snapshot"),
        &cookie,
        &snapshot_settings,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(
        snapshot, repeated_snapshot,
        "same snapshot inputs are deterministic"
    );

    let empty_history: Value = get(&client, &base, "/api/client/historical/10", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty_history, json!([]));

    let summary: Value = get(&client, &base, "/api/v1/analytics/summary", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summary["customersTotal"], 4);
    assert_eq!(summary["expectedLossHorizonDays"], 30);
    let customers: Value = get(
        &client,
        &base,
        "/api/v1/customers?sort=id&direction=asc&limit=10",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    let mut assessed_revenue = 0_i64;
    let mut unassessed_revenue = 0_i64;
    let mut expected_loss = 0_i64;
    let mut high_risk_revenue = 0_i64;
    let mut payment_deficit = 0_i64;
    let mut unassessed_customers = 0_i64;
    for item in customers["items"].as_array().unwrap() {
        let account_id = item["account"]["id"].as_i64().unwrap();
        let detail: Value = get(
            &client,
            &base,
            &format!("/api/v1/customers/{account_id}"),
            &cookie,
        )
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
        let monthly_revenue: i64 = detail["services"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|service| matches!(service["status"].as_str(), Some("active" | "suspended")))
            .map(|service| service["price"].as_i64().unwrap())
            .sum();
        let risk = &item["risk"];
        if risk["assessmentStatus"] == "sufficient_data" {
            assessed_revenue += monthly_revenue;
            expected_loss += (monthly_revenue * risk["score"].as_i64().unwrap() + 50) / 100;
        } else {
            unassessed_revenue += monthly_revenue;
            unassessed_customers += 1;
        }
        if risk["level"] == "high" {
            high_risk_revenue += monthly_revenue;
        }
        payment_deficit += (risk["upcomingCents"].as_i64().unwrap()
            - risk["spendableBalanceCents"].as_i64().unwrap())
        .max(0);
    }
    assert_eq!(summary["expectedLossEurCents"], expected_loss);
    assert_eq!(
        summary["expectedLossAssessedMonthlyRevenueEurCents"],
        assessed_revenue
    );
    assert_eq!(
        summary["expectedLossUnassessedMonthlyRevenueEurCents"],
        unassessed_revenue
    );
    assert_eq!(
        summary["expectedLossUnassessedCustomers"],
        unassessed_customers
    );
    assert_eq!(summary["revenueAtRiskEurCents"], high_risk_revenue);
    assert_eq!(summary["paymentDeficitEurCents"], payment_deficit);

    // The market median is withheld until two distinct, fully comparable
    // providers are present. FX uses the source observation date.
    let first_observed = Utc::now() - ChronoDuration::days(3);
    let second_observed = first_observed + ChronoDuration::days(1);
    let first_date = first_observed.date_naive();
    let second_date = second_observed.date_naive();
    for date in [first_date, second_date] {
        for (base_currency, rate) in [("EUR", 100.0_f64), ("USD", 80.0_f64)] {
            sqlx::query(
                "INSERT INTO market_intelligence.exchange_rates \
                 (base, quote, requested_on, rate_on, rate, source) \
                 VALUES ($1, 'RUB', $2, $2, $3, 'integration fixture') \
                 ON CONFLICT (base, quote, requested_on) DO UPDATE SET rate = excluded.rate, rate_on = excluded.rate_on",
            )
            .bind(base_currency)
            .bind(date)
            .bind(rate)
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    let market_source = |observed_at, source_url: &str| ObservationSource {
        kind: "live",
        observed_at,
        source_url: source_url.into(),
        archive_url: None,
        cdx_digest: None,
        parser_version: "market-api-fixture-v1",
    };
    let market_offer =
        |key: &str, price_minor, currency: &str, region: &str, vat, traffic: &str| Offer {
            plan_key: key.into(),
            plan_name: key.into(),
            cpu_cores: Some(2),
            ram_mib: Some(4096),
            disk_gib: Some(80),
            disk_type: Some("NVMe".into()),
            region: Some(region.into()),
            price_minor: Some(price_minor),
            currency: Some(currency.into()),
            billing_period: Some("month".into()),
            price_kind: "regular".into(),
            vat_included: Some(vat),
            raw_excerpt: format!("2 vCPU; 4 GB RAM; 80 GB NVMe; Traffic: {traffic}"),
        };
    save_offers(
        &pool,
        "aeza",
        &market_source(second_observed, "https://aeza.test/benchmark"),
        &[market_offer(
            "benchmark-aeza",
            700_000,
            "RUB",
            "Frankfurt",
            true,
            "unlimited",
        )],
    )
    .await
    .unwrap();
    save_offers(
        &pool,
        "contabo",
        &market_source(first_observed, "https://contabo.test/benchmark"),
        &[market_offer(
            "benchmark-contabo",
            5_500,
            "EUR",
            "DE-FRA",
            true,
            "unlimited",
        )],
    )
    .await
    .unwrap();
    let without_analogs: Value = get(&client, &base, "/api/market/prices/monitor", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let own_without_analogs = without_analogs
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["providerCode"] == "aeza" && row["planKey"] == "benchmark-aeza")
        .unwrap();
    assert_eq!(own_without_analogs["marketMedianRub"], Value::Null);
    assert_eq!(own_without_analogs["providersAtMedian"], 0);

    save_offers(
        &pool,
        "hetzner",
        &market_source(second_observed, "https://hetzner.test/benchmark"),
        &[market_offer(
            "benchmark-hetzner",
            10_000,
            "USD",
            "Frankfurt",
            true,
            "unlimited",
        )],
    )
    .await
    .unwrap();
    save_offers(
        &pool,
        "firstvds",
        &market_source(second_observed, "https://firstvds.test/region-mismatch"),
        &[market_offer(
            "benchmark-wrong-region",
            1,
            "RUB",
            "London",
            true,
            "unlimited",
        )],
    )
    .await
    .unwrap();
    save_offers(
        &pool,
        "selectel",
        &market_source(
            second_observed,
            "https://selectel.test/traffic-vat-mismatch",
        ),
        &[market_offer(
            "benchmark-wrong-terms",
            1,
            "RUB",
            "Frankfurt",
            false,
            "1 TB",
        )],
    )
    .await
    .unwrap();
    let mut malformed_price = market_offer(
        "benchmark-malformed-price",
        0,
        "RUB",
        "Moscow",
        true,
        "Traffic: 1 TB",
    );
    malformed_price.price_minor = None;
    save_offers(
        &pool,
        "firstvds",
        &market_source(second_observed, "https://firstvds.test/malformed-price"),
        &[malformed_price],
    )
    .await
    .unwrap();
    save_offers(
        &pool,
        "contabo",
        &market_source(second_observed, "https://contabo.test/benchmark"),
        &[market_offer(
            "benchmark-contabo",
            6_000,
            "EUR",
            "DE-FRA",
            true,
            "unlimited",
        )],
    )
    .await
    .unwrap();
    let monitored: Value = get(&client, &base, "/api/market/prices/monitor", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let own = monitored
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["providerCode"] == "aeza" && row["planKey"] == "benchmark-aeza")
        .unwrap();
    assert_eq!(own["marketMedianRub"], 7000.0);
    assert_eq!(
        own["providersAtMedian"], 2,
        "region, VAT and traffic mismatches are excluded"
    );
    let contabo = monitored
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["providerCode"] == "contabo" && row["planKey"] == "benchmark-contabo")
        .unwrap();
    assert_eq!(contabo["previousPriceRub"], 5500.0);
    assert_eq!(contabo["priceRub"], 6000.0);
    assert!((contabo["changePercent"].as_f64().unwrap() - 100.0 / 11.0).abs() < 1e-9);
    assert_eq!(contabo["priceEur"], 60.0);
    assert_eq!(contabo["priceUsd"], 75.0);
    let benchmark_history: Value = get(
        &client,
        &base,
        "/api/market/prices/benchmarks/history",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    let own_history = benchmark_history
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["planKey"] == "benchmark-aeza")
        .unwrap();
    assert_eq!(own_history["marketMedianRub"], 7000.0);
    assert_eq!(own_history["providersAtMedian"], 2);
    let price_history: Value = get(&client, &base, "/api/market/prices/history", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(price_history.as_array().unwrap().iter().any(|row| {
        row["providerCode"] == "contabo"
            && row["planKey"] == "benchmark-contabo"
            && row["priceRub"] == 6000.0
    }));

    let prices: Value = get(&client, &base, "/api/market/prices", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let contabo_price = prices
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["planName"] == "benchmark-contabo")
        .unwrap();
    assert_eq!(contabo_price["originalMinor"], 6000);
    assert_eq!(contabo_price["currency"], "EUR");
    assert_eq!(contabo_price["priceRubMinor"], 600000);
    assert!(
        !prices
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["planName"] == "benchmark-malformed-price")
    );

    let quality: Value = get(&client, &base, "/api/market/prices/quality", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let contabo_quality = quality
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["providerCode"] == "contabo")
        .unwrap();
    assert_eq!(contabo_quality["products"], 1);
    assert_eq!(contabo_quality["configurations"], 1);
    assert_eq!(contabo_quality["coreSpecsKnown"], 1);
    assert_eq!(contabo_quality["regionsKnown"], 1);
    assert_eq!(contabo_quality["diskTypeKnown"], 1);
    assert_eq!(contabo_quality["trafficKnown"], 1);
    assert_eq!(contabo_quality["completePrices"], 2);
    assert_eq!(contabo_quality["missingFxPrices"], 0);

    let changes: Value = get(&client, &base, "/api/market/prices/changes", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let contabo_change = changes
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["providerCode"] == "contabo" && row["planKey"] == "benchmark-contabo")
        .unwrap();
    assert_eq!(contabo_change["previousPriceMinor"], 5500);
    assert_eq!(contabo_change["currentPriceMinor"], 6000);
    assert_eq!(contabo_change["currency"], "EUR");
    assert!((contabo_change["changePercent"].as_f64().unwrap() - 100.0 / 11.0).abs() < 1e-9);

    let sources: Value = get(&client, &base, "/api/market/prices/sources", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let firstvds_source = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["providerCode"] == "firstvds")
        .unwrap();
    assert_eq!(firstvds_source["completeObservations"], 1);
    assert_eq!(firstvds_source["incompleteObservations"], 1);
    assert!(firstvds_source["lastCompleteAt"].is_string());
    let timeweb_source = sources
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["providerCode"] == "timeweb_cloud")
        .unwrap();
    assert_eq!(timeweb_source["completeObservations"], 0);
    assert!(timeweb_source["lastCompleteAt"].is_null());

    let settings: Value = get(&client, &base, "/api/v1/model/settings", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(settings["metadata"]["version"], 0);
    assert_eq!(
        patch_json(
            &client,
            format!("{base}/api/v1/model/settings"),
            None,
            &settings["settings"],
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut invalid_settings = settings["settings"].clone();
    invalid_settings["weights"]["paymentCoverage"] = json!(101.0);
    assert_eq!(
        patch_json(
            &client,
            format!("{base}/api/v1/model/settings"),
            Some(&cookie),
            &invalid_settings,
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    for (field, invalid_value) in [
        ("paymentCoverage", -1),
        ("noAutoProlong", -1),
        ("prolongWait", -1),
        ("failedProlong", -1),
        ("expiringRevenue", -1),
        ("priceRisk", -1),
        ("paymentCoverage", 101),
        ("noAutoProlong", 101),
        ("prolongWait", 101),
        ("failedProlong", 101),
        ("expiringRevenue", 101),
        ("priceRisk", 101),
    ] {
        let mut invalid = settings["settings"].clone();
        invalid["weights"][field] = json!(invalid_value);
        assert_eq!(
            patch_json(
                &client,
                format!("{base}/api/v1/model/settings"),
                Some(&cookie),
                &invalid,
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST,
            "invalid weight {field}={invalid_value} must be rejected"
        );
    }
    let mut all_weights_zero = settings["settings"].clone();
    for field in [
        "paymentCoverage",
        "noAutoProlong",
        "prolongWait",
        "failedProlong",
        "expiringRevenue",
        "priceRisk",
    ] {
        all_weights_zero["weights"][field] = json!(0);
    }
    let mut invalid_thresholds = settings["settings"].clone();
    invalid_thresholds["highThreshold"] = invalid_thresholds["mediumThreshold"].clone();
    let mut invalid_window = settings["settings"].clone();
    invalid_window["renewalWindowDays"] = json!(0);
    let mut invalid_history = settings["settings"].clone();
    invalid_history["historicalMinDays"] = json!(6);
    for invalid in [
        all_weights_zero,
        invalid_thresholds,
        invalid_window,
        invalid_history,
    ] {
        assert_eq!(
            patch_json(
                &client,
                format!("{base}/api/v1/model/settings"),
                Some(&cookie),
                &invalid,
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST,
            "inconsistent or zero-range settings must be rejected"
        );
    }
    assert_eq!(
        client
            .patch(format!("{base}/api/v1/model/settings"))
            .header(reqwest::header::COOKIE, &cookie)
            .header(reqwest::header::ORIGIN, "http://attacker.invalid")
            .json(&settings["settings"])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut updated_settings = settings["settings"].clone();
    updated_settings["mediumThreshold"] = json!(40);
    let updated: Value = patch_json(
        &client,
        format!("{base}/api/v1/model/settings"),
        Some(&cookie),
        &updated_settings,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(updated["settings"]["mediumThreshold"], 40);
    assert_eq!(updated["metadata"]["version"], 1);
    let rescored: Value = get(
        &client,
        &base,
        "/api/v1/customers?sort=id&direction=asc&limit=10",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    for customer in rescored["items"].as_array().unwrap() {
        let id = customer["account"]["id"].as_i64().unwrap();
        let current_risk = &customer["risk"];
        let old_risk = snapshot
            .as_array()
            .unwrap()
            .iter()
            .find(|risk| risk["accountId"] == id.to_string())
            .unwrap();
        let score = current_risk["score"].as_i64().unwrap();
        assert_eq!(
            score, old_risk["score"],
            "threshold changes do not alter score"
        );
        assert_eq!(current_risk["settingsVersion"], 1);
        let expected_level = match current_risk["assessmentStatus"].as_str().unwrap() {
            "insufficient_data" => "insufficient",
            "limited_data" => "limited",
            _ if score >= updated["settings"]["highThreshold"].as_i64().unwrap() => "high",
            _ if score >= updated["settings"]["mediumThreshold"].as_i64().unwrap() => "medium",
            _ => "low",
        };
        assert_eq!(current_risk["level"], expected_level);
    }

    let metrics = get(&client, &base, "/metrics", &cookie).await;
    assert_eq!(metrics.status(), StatusCode::OK);
    assert!(
        metrics.headers()[reqwest::header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    let metrics_body = metrics.text().await.unwrap();
    assert!(metrics_body.contains("# TYPE aeza_customers_total gauge"));
    assert!(metrics_body.contains("aeza_customers_total 4"));
    let summary: Value = get(&client, &base, "/api/v1/analytics/summary", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    for (json_field, metric_name) in [
        ("customersTotal", "aeza_customers_total"),
        ("revenueAtRiskEurCents", "aeza_revenue_at_risk_eur_cents"),
        ("paymentDeficitEurCents", "aeza_payment_deficit_eur_cents"),
        ("expectedLossEurCents", "aeza_expected_loss_30d_eur_cents"),
        (
            "expectedLossAssessedMonthlyRevenueEurCents",
            "aeza_expected_loss_assessed_mrr_eur_cents",
        ),
        (
            "expectedLossUnassessedMonthlyRevenueEurCents",
            "aeza_expected_loss_unassessed_mrr_eur_cents",
        ),
        (
            "expectedLossUnassessedCustomers",
            "aeza_expected_loss_unassessed_customers",
        ),
    ] {
        let expected = summary[json_field].as_u64().unwrap();
        assert!(
            metrics_body.contains(&format!("{metric_name} {expected}\n")),
            "Prometheus {metric_name} should match summary.{json_field}"
        );
    }
    assert!(
        !metrics_body.contains("account_id") && !metrics_body.contains("customer_id"),
        "metrics must not expose per-customer labels"
    );
    for level in ["high", "medium", "low", "limited", "insufficient"] {
        let expected = summary["customersByRisk"][level].as_u64().unwrap_or(0);
        assert!(metrics_body.contains(&format!(
            "aeza_customers_by_risk{{level=\"{level}\"}} {expected}\n"
        )));
    }
    let docs = get(&client, &base, "/api/v1/docs", &cookie).await;
    assert_eq!(docs.status(), StatusCode::OK);
    assert!(docs.text().await.unwrap().contains("SwaggerUIBundle"));
    let served_openapi: Value = client
        .get(format!("{base}/api/v1/openapi.json"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(served_openapi["openapi"], "3.0.3");
    for path in [
        "/api/v1/auth/login",
        "/api/v1/auth/me",
        "/api/v1/auth/logout",
        "/api/v1/customers",
        "/api/v1/customers/{id}",
        "/api/v1/analytics/summary",
        "/api/v1/model/settings",
        "/api/v1/source/snapshot",
    ] {
        assert!(
            served_openapi["paths"][path].is_object(),
            "OpenAPI is missing {path}"
        );
    }

    let logout = client
        .post(format!("{base}/api/v1/auth/logout"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::OK);
    assert_eq!(
        get(&client, &base, "/api/v1/auth/me", &cookie)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    drop(server);

    // Settings and source data survive a complete API process restart.
    let mut server = ApiServer::start(database_url, &address);
    ready(&client, &base, &mut server).await;
    let cookie = login(&client, &base).await;
    let persisted: Value = get(&client, &base, "/api/v1/model/settings", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(persisted["settings"]["mediumThreshold"], 40);
    assert_eq!(persisted["metadata"]["version"], 1);

    // A settings-history insert failure must roll back the primary settings row
    // and leave the in-memory model unchanged.
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION public.aeza_v1_api_reject_settings_history() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'forced settings save failure'; END $$; \
         CREATE TRIGGER aeza_v1_api_reject_settings_history BEFORE INSERT \
         ON public.client_settings_history FOR EACH ROW \
         EXECUTE FUNCTION public.aeza_v1_api_reject_settings_history()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut failed_settings = persisted["settings"].clone();
    failed_settings["mediumThreshold"] = json!(50);
    assert_eq!(
        patch_json(
            &client,
            format!("{base}/api/v1/model/settings"),
            Some(&cookie),
            &failed_settings,
        )
        .await
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    sqlx::raw_sql(
        "DROP TRIGGER aeza_v1_api_reject_settings_history ON public.client_settings_history; \
         DROP FUNCTION public.aeza_v1_api_reject_settings_history()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let after_failed_save: Value = get(&client, &base, "/api/v1/model/settings", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_failed_save["settings"]["mediumThreshold"], 40);
    assert_eq!(after_failed_save["metadata"]["version"], 1);

    // Concurrent valid updates serialize into complete, monotonically
    // versioned configurations rather than partially mixed settings.
    let mut updates = tokio::task::JoinSet::new();
    for threshold in [41, 42, 43, 44] {
        let client = client.clone();
        let url = format!("{base}/api/v1/model/settings");
        let cookie = cookie.clone();
        let mut settings = persisted["settings"].clone();
        settings["mediumThreshold"] = json!(threshold);
        updates.spawn(async move { patch_json(&client, url, Some(&cookie), &settings).await });
    }
    let mut versions = Vec::new();
    while let Some(result) = updates.join_next().await {
        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        versions.push(body["metadata"]["version"].as_i64().unwrap());
        let threshold = body["settings"]["mediumThreshold"].as_i64().unwrap();
        assert!((41..=44).contains(&threshold));
        assert_eq!(body["settings"]["highThreshold"], 65);
    }
    versions.sort_unstable();
    assert_eq!(versions, [2, 3, 4, 5]);
    let after_concurrent_updates: Value = get(&client, &base, "/api/v1/model/settings", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_concurrent_updates["metadata"]["version"], 5);
    assert!(
        (41..=44).contains(
            &after_concurrent_updates["settings"]["mediumThreshold"]
                .as_i64()
                .unwrap()
        )
    );
    let history_versions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.client_settings_history WHERE version BETWEEN 2 AND 5",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(history_versions, 4);

    // Force a PostgreSQL insert failure after the replacement transaction has
    // deleted the old source rows; the API must return an error and rollback.
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION public.aeza_v1_api_test_reject_account() RETURNS trigger \
         LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'forced API snapshot failure'; END $$; \
         CREATE TRIGGER aeza_v1_api_test_reject_account BEFORE INSERT ON public.accounts \
         FOR EACH ROW EXECUTE FUNCTION public.aeza_v1_api_test_reject_account()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let failed_replacement = serde_json::to_value(fixture_for(&[501])).unwrap();
    assert_eq!(
        put_json(
            &client,
            format!("{base}/api/v1/source/snapshot"),
            &cookie,
            &failed_replacement,
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    sqlx::raw_sql(
        "DROP TRIGGER aeza_v1_api_test_reject_account ON public.accounts; \
         DROP FUNCTION public.aeza_v1_api_test_reject_account()",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "DELETE FROM public.admin_sessions WHERE token_hash = sha256(convert_to($1, 'UTF8'))",
    )
    .bind("isolated-viewer-session-token")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "DELETE FROM public.admin_sessions WHERE token_hash = sha256(convert_to($1, 'UTF8'))",
    )
    .bind("isolated-expired-session-token")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM public.admins WHERE email = $1")
        .bind("v1-api-expired@aeza.test")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM public.admins WHERE email = $1")
        .bind("v1-api-viewer@aeza.test")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let after_failed_replacement: Value = get(
        &client,
        &base,
        "/api/v1/customers?sort=id&limit=10",
        &cookie,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(after_failed_replacement["total"], 4);
    assert_eq!(after_failed_replacement["items"][0]["account"]["id"], 2);

    let mut broken_relationship = serde_json::to_value(fixture_for(&[501])).unwrap();
    broken_relationship["services"][0]["ownerId"] = json!(999_999);
    let mut duplicate_account = serde_json::to_value(fixture_for(&[501])).unwrap();
    let duplicate_row = duplicate_account["accounts"][0].clone();
    duplicate_account["accounts"]
        .as_array_mut()
        .unwrap()
        .push(duplicate_row);
    let mut unknown_service_status = serde_json::to_value(fixture_for(&[501])).unwrap();
    unknown_service_status["services"][0]["status"] = json!("running");
    let mut negative_amount = serde_json::to_value(fixture_for(&[501])).unwrap();
    negative_amount["transactions"][0]["amount"] = json!(-1);
    let malformed_snapshots = [
        (broken_relationship, StatusCode::BAD_REQUEST),
        (duplicate_account, StatusCode::BAD_REQUEST),
        (unknown_service_status, StatusCode::UNPROCESSABLE_ENTITY),
        (negative_amount, StatusCode::BAD_REQUEST),
        (
            json!({"accounts":[{"id":"wrong-type"}],"services":[],"transactions":[]}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ];
    for (invalid_snapshot, expected_status) in malformed_snapshots {
        assert_eq!(
            put_json(
                &client,
                format!("{base}/api/v1/source/snapshot"),
                &cookie,
                &invalid_snapshot,
            )
            .await
            .status(),
            expected_status,
            "invalid dataset input should fail before deleting the old snapshot"
        );
        assert_eq!(
            get(
                &client,
                &base,
                "/api/v1/customers?sort=id&limit=10",
                &cookie
            )
            .await
            .json::<Value>()
            .await
            .unwrap()["total"],
            4
        );
    }

    let empty_dataset = json!({"accounts": [], "services": [], "transactions": []});
    assert_eq!(
        put_json(
            &client,
            format!("{base}/api/v1/source/snapshot"),
            &cookie,
            &empty_dataset,
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let mut live_sockets: Vec<_> = (0..32)
        .map(|_| TestWebSocket::connect(&address, &cookie))
        .collect();
    for socket in &mut live_sockets {
        let initial_live_state = socket.recv_json();
        assert_eq!(initial_live_state["type"], "snapshot");
        assert_eq!(
            initial_live_state["dataset"]["accounts"]
                .as_array()
                .unwrap()
                .len(),
            4,
            "a newly connected WebSocket receives the current dataset"
        );
    }
    let replacement = fixture_for(&[500]);
    let replacement_value = serde_json::to_value(replacement).unwrap();
    let replaced: Value = put_json(
        &client,
        format!("{base}/api/v1/source/snapshot"),
        &cookie,
        &replacement_value,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(replaced["accounts"], 1);
    assert_eq!(replaced["services"], 1);
    assert_eq!(replaced["transactions"], 1);
    let dataset_event = live_sockets[0].recv_json();
    assert_eq!(dataset_event["type"], "dataset");
    assert_eq!(
        dataset_event["dataset"]["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(dataset_event["dataset"]["accounts"][0]["id"], "500");
    let scores_event = live_sockets[0].recv_json();
    assert_eq!(scores_event["type"], "scores");
    assert_eq!(scores_event["changed"].as_array().unwrap().len(), 1);
    assert_eq!(scores_event["changed"][0]["accountId"], "500");
    let mut removed_ids: Vec<_> = scores_event["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    removed_ids.sort_unstable();
    assert_eq!(removed_ids, ["10", "100", "11", "2"]);
    for socket in live_sockets.iter_mut().skip(1) {
        assert_eq!(
            socket.recv_json(),
            dataset_event,
            "all open tabs receive the same dataset"
        );
        assert_eq!(
            socket.recv_json(),
            scores_event,
            "all open tabs receive the same score update"
        );
    }
    live_sockets[0].assert_no_message_for(Duration::from_millis(200));
    drop(live_sockets);
    let replayed: Value = put_json(
        &client,
        format!("{base}/api/v1/source/snapshot"),
        &cookie,
        &replacement_value,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(replayed, replaced, "replaying a dataset is deterministic");
    let customers: Value = get(&client, &base, "/api/v1/customers?sort=id", &cookie)
        .await
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(customers["total"], 1);
    assert_eq!(customers["items"][0]["account"]["id"], 500);
    assert_eq!(
        get(&client, &base, "/api/v1/customers/2", &cookie)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    // Break only the account-table reads while leaving PostgreSQL online, then
    // ensure database failures surface as 500 and requests recover afterwards.
    let failure_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE public.accounts RENAME TO aeza_v1_api_accounts_unavailable")
        .execute(&failure_pool)
        .await
        .unwrap();
    let list_failure = get(&client, &base, "/api/v1/customers?sort=id", &cookie)
        .await
        .status();
    let detail_failure = get(&client, &base, "/api/v1/customers/500", &cookie)
        .await
        .status();
    sqlx::query("ALTER TABLE public.aeza_v1_api_accounts_unavailable RENAME TO accounts")
        .execute(&failure_pool)
        .await
        .unwrap();
    failure_pool.close().await;
    assert_eq!(list_failure, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(detail_failure, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        get(&client, &base, "/api/v1/customers?sort=id", &cookie)
            .await
            .status(),
        StatusCode::OK,
        "API recovers after the database relation is restored"
    );

    // Load a larger deterministic fixture, restart the API, and exercise the
    // actual list/detail handlers concurrently plus a complete Risk Score run.
    drop(server);
    let stress_ids: Vec<i64> = (10_000..11_200).collect();
    let stress_dataset = fixture_for(&stress_ids);
    let stress_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .unwrap();
    PgAdapter::from_pool(stress_pool.clone())
        .replace(&stress_dataset)
        .await
        .unwrap();
    stress_pool.close().await;

    let mut server = ApiServer::start(database_url, &address);
    ready(&client, &base, &mut server).await;
    let cookie = login(&client, &base).await;
    let started = Instant::now();
    let mut reads = tokio::task::JoinSet::new();
    for page_number in 1..=8 {
        let client = client.clone();
        let base = base.clone();
        let cookie = cookie.clone();
        reads.spawn(async move {
            let page: Value = get(
                &client,
                &base,
                &format!("/api/v1/customers?sort=id&page={page_number}&limit=100"),
                &cookie,
            )
            .await
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
            assert_eq!(page["total"], 1200);
            assert_eq!(page["items"].as_array().unwrap().len(), 100);
        });
    }
    for index in 0..8 {
        let client = client.clone();
        let base = base.clone();
        let cookie = cookie.clone();
        reads.spawn(async move {
            let id = 10_000 + index;
            assert_eq!(
                get(&client, &base, &format!("/api/v1/customers/{id}"), &cookie)
                    .await
                    .status(),
                StatusCode::OK
            );
        });
    }
    while let Some(result) = reads.join_next().await {
        result.unwrap();
    }
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "parallel list/detail responses exceeded 30 seconds"
    );

    let recalculation_started = Instant::now();
    let recalculated: Value = post_json(
        &client,
        format!("{base}/api/client/snapshot"),
        &cookie,
        &snapshot_settings,
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(recalculated.as_array().unwrap().len(), 1200);
    assert!(recalculated.as_array().unwrap().iter().all(|risk| {
        risk["score"]
            .as_i64()
            .is_some_and(|score| (0..=100).contains(&score))
    }));
    assert!(
        recalculation_started.elapsed() < Duration::from_secs(30),
        "full risk recalculation exceeded 30 seconds"
    );
}

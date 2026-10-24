mod support;
use aeza_market_simulator::*;
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};
use support::*;

#[test]
fn economic_engine_matrix() {
    let mut rows = vec![];
    let mut reports = BTreeMap::new();
    for case in cases() {
        let mut errors = vec![];
        match simulate(case.scenario.clone(),case.market.clone()){
   Ok(report)=>{errors.extend(verify(&case,&report));let replay=simulate(case.scenario.clone(),case.market.clone()).unwrap();if serde_json::to_vec(&report).unwrap()!=serde_json::to_vec(&replay).unwrap(){errors.push("nonidentical seeded replay".into());}
    rows.push(json!({"id":case.id,"transport":"engine","expected":case.expected,"passed":errors.is_empty(),"errors":errors,"actual":observation(&report)}));reports.insert(case.id,report);
   }
   Err(error)=>rows.push(json!({"id":case.id,"transport":"engine","expected":case.expected,"passed":false,"errors":[error]})),
  }
    }
    rows.extend(relational(&reports));
    save("engine-results.json", &rows);
    let failures: Vec<_> = rows
        .iter()
        .filter(|r| r["passed"] != true)
        .map(|r| json!({"id":r["id"],"errors":r["errors"]}))
        .collect();
    assert!(
        failures.is_empty(),
        "economic engine failures: {failures:#?}"
    );
    println!(
        "{} engine economic cases and relational assertions passed",
        rows.len()
    );
}
fn request(addr: SocketAddr, path: &str, body: Option<&[u8]>) -> Result<(u16, Vec<u8>), String> {
    let mut conn =
        TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
    conn.set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|e| e.to_string())?;
    let data = body.unwrap_or_default();
    let method = if body.is_some() { "POST" } else { "GET" };
    write!(conn,"{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",data.len()).map_err(|e|e.to_string())?;
    conn.write_all(data).map_err(|e| e.to_string())?;
    let mut raw = vec![];
    conn.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let split = raw
        .windows(4)
        .position(|v| v == b"\r\n\r\n")
        .ok_or("missing HTTP headers")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let status = head
        .split_whitespace()
        .nth(1)
        .ok_or("no status")?
        .parse()
        .map_err(|_| "invalid status")?;
    let body = &raw[split + 4..];
    if head.to_lowercase().contains("transfer-encoding: chunked") {
        let mut remaining = body;
        let mut decoded = vec![];
        loop {
            let line = remaining
                .windows(2)
                .position(|v| v == b"\r\n")
                .ok_or("bad chunk size")?;
            let size = usize::from_str_radix(
                std::str::from_utf8(&remaining[..line])
                    .map_err(|e| e.to_string())?
                    .split(';')
                    .next()
                    .unwrap(),
                16,
            )
            .map_err(|e| e.to_string())?;
            remaining = &remaining[line + 2..];
            if size == 0 {
                break;
            }
            if remaining.len() < size + 2 {
                return Err("truncated chunk".into());
            }
            decoded.extend_from_slice(&remaining[..size]);
            remaining = &remaining[size + 2..];
        }
        Ok((status, decoded))
    } else {
        Ok((status, body.to_vec()))
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn economic_http_tcp_matrix() {
    let mut rows = vec![];
    let mut reports = BTreeMap::new();
    for case in cases() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local TCP permission required for real HTTP tests");
        let addr = listener.local_addr().unwrap();
        let app = api::router_with_catalog(case.market.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let scenario = case.scenario.clone();
        let catalog = case.market.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<(Report, bool), String> {
            let (status, body) = request(addr, "/api/market/v1/catalog", None)?;
            if status != 200 {
                return Err(format!("catalog status {status}"));
            }
            let actual: serde_json::Value =
                serde_json::from_slice(&body).map_err(|e| e.to_string())?;
            if actual != serde_json::to_value(catalog).unwrap() {
                return Err("HTTP catalog differs from supplied controlled catalog".into());
            }
            let (source_status, source_body) = request(addr, "/api/market/v1/sources", None)?;
            let sources: serde_json::Value =
                serde_json::from_slice(&source_body).map_err(|e| e.to_string())?;
            if source_status != 200
                || sources["catalog_included"].as_u64()
                    != Some(actual.as_array().unwrap().len() as u64)
            {
                return Err("injected catalog provenance mismatch".into());
            }
            let payload = serde_json::to_vec(&scenario).unwrap();
            let (status, body) = request(addr, "/api/market/v1/simulate", Some(&payload))?;
            if status != 200 {
                return Err(format!("HTTP {status}: {}", String::from_utf8_lossy(&body)));
            }
            let (second_status, second) = request(addr, "/api/market/v1/simulate", Some(&payload))?;
            let report: Report = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
            Ok((report, status == second_status && body == second))
        })
        .await
        .unwrap();
        server.abort();
        let _ = server.await;
        match result {Ok((report,replay))=>{let mut errors=verify(&case,&report);if !replay{errors.push("HTTP replay bytes differ".into())}let engine=simulate(case.scenario.clone(),case.market.clone()).unwrap();if serde_json::to_value(&report).unwrap()!=serde_json::to_value(engine).unwrap(){errors.push("HTTP response differs from direct engine result".into())}
   rows.push(json!({"id":case.id,"transport":"real_tcp_http","expected":case.expected,"passed":errors.is_empty(),"errors":errors,"actual":observation(&report)}));reports.insert(case.id,report);
  },Err(error)=>rows.push(json!({"id":case.id,"transport":"real_tcp_http","expected":case.expected,"passed":false,"errors":[error]}))}
    }
    // Preserve all collected failures before panicking; missing reports are reported explicitly.
    if reports.len() == cases().len() {
        rows.extend(relational(&reports));
    }
    save("http-results.json", &rows);
    let failures: Vec<_> = rows
        .iter()
        .filter(|r| r["passed"] != true)
        .map(|r| json!({"id":r["id"],"errors":r["errors"]}))
        .collect();
    assert!(failures.is_empty(), "economic HTTP failures: {failures:#?}");
    println!(
        "{} HTTP economic cases and relational assertions passed",
        rows.len()
    );
}

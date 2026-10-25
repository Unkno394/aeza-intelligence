use aeza_market_simulator::{api::router, example};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use tower::ServiceExt;
#[tokio::test]
async fn read_endpoints() {
    for p in ["health", "catalog", "sources", "example", "openapi.json"] {
        let r = router()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/market/v1/{p}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        serde_json::from_slice::<serde_json::Value>(
            &r.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap();
    }
}
#[tokio::test]
async fn simulate_and_validation() {
    for (body, status) in [
        (
            serde_json::to_string(&{
                let mut s = example();
                s.buyers_per_month = 2;
                s
            })
            .unwrap(),
            StatusCode::OK,
        ),
        ("{}".into(), StatusCode::UNPROCESSABLE_ENTITY),
        ("{".into(), StatusCode::BAD_REQUEST),
        (
            serde_json::to_string(&{
                let mut s = example();
                s.months = 0;
                s
            })
            .unwrap(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            serde_json::to_string(&{
                let mut s = example();
                s.aeza_plans[0].monthly_usd = 0.0;
                s
            })
            .unwrap(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let r = router()
            .oneshot(
                Request::post("/api/market/v1/simulate")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), status);
        let v: serde_json::Value =
            serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
        if status == StatusCode::OK {
            assert_eq!(v["runs"].as_array().unwrap().len(), 12);
        } else {
            assert!(v["error"].is_string());
        }
    }
}
#[tokio::test]
async fn oversized_body_is_rejected() {
    let r = router()
        .oneshot(
            Request::post("/api/market/v1/simulate")
                .header("content-type", "application/json")
                .body(Body::from(" ".repeat(129 * 1024)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE)
}

#[tokio::test]
async fn rub_v2_defaults_to_real_available_aeza_and_keeps_contexts_separate() {
    let app = router();
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/market/v2/example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["currency"], "RUB");
    assert_eq!(value["market"], "russia");
    let plans = value["aeza_plans"].as_array().unwrap();
    assert_eq!(plans.len(), 3);
    assert!(plans.iter().all(|p| p["origin"] == "observed_list_price"
        && p["details"]["availability"] == "available_from_source"
        && p["monthly_rub"].as_f64().unwrap() > 0.));
    let r = app
        .oneshot(
            Request::builder()
                .uri("/api/market/v2/contexts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["gpu"]["usage"], "reference_only_gpu_not_vps");
    assert!(value["akash"]["daily_points"].as_u64().unwrap() > 2000);
}

#[tokio::test]
async fn statistics_api_rub_ties_and_invalid_inputs() {
    use serde_json::{Value, json};
    async fn call(path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let request = if let Some(body) = body {
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        } else {
            Request::get(path).body(Body::empty()).unwrap()
        };
        let r = router().oneshot(request).await.unwrap();
        let status = r.status();
        let value =
            serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
        (status, value)
    }
    let (_, mut s) = call("/api/market/v2/example", None).await;
    s["buyers_per_month"] = json!(0);
    let payload = json!({"scenario":s,"sensitivity":"medium"});
    let (status, a) = call("/api/market/v2/statistics", Some(payload.clone())).await;
    assert_eq!(status, StatusCode::OK);
    let (_, b) = call("/api/market/v2/statistics", Some(payload)).await;
    assert_eq!(a, b);
    assert_eq!(a["currency"], "RUB");
    assert_eq!(a["seed_count"], 100);
    assert_eq!(a["tied_seed_count"], 100);
    assert_eq!(a["seeds"][99], 99);
    for strategy in a["strategies"].as_array().unwrap() {
        assert_eq!(strategy["sole_wins"], 0);
        assert_eq!(strategy["tied_wins"], 100);
        assert_eq!(strategy["loss_runs"], 100);
        assert!(strategy["profit"]["min_rub"].as_f64().unwrap() < 0.);
        assert!(strategy["profit"].get("mean_rub").is_none());
        assert_eq!(strategy["points"].as_array().unwrap().len(), 100);
    }
    for body in [
        json!({}),
        json!({"scenario":s,"sensitivity":"invalid"}),
        json!({"scenario":s,"sensitivity":"medium","seed_count":10}),
    ] {
        assert_eq!(
            call("/api/market/v2/statistics", Some(body)).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    s["valuation_fx"]["rub_per_unit"]["USD"] = json!("999");
    assert_eq!(
        call(
            "/api/market/v2/statistics",
            Some(json!({"scenario":s,"sensitivity":"medium"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

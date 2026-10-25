use crate::{Scenario, simulate};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone)]
struct ApiState {
    slots: Arc<Semaphore>,
    statistics_slots: Arc<Semaphore>,
    market: Arc<Vec<crate::Plan>>,
    sources: Arc<Value>,
    store: Option<Arc<crate::market_data::Store>>,
}
pub fn router() -> Router {
    router_with_data_path(crate::market_data::default_path())
}
pub fn router_with_data_path(path: std::path::PathBuf) -> Router {
    let store = Arc::new(crate::market_data::Store::new(path));
    let (snapshot, error) = store.current();
    build_router(
        snapshot.plans.clone(),
        sources(&snapshot, error),
        Some(store),
    )
}
fn sources(s: &crate::market_data::Snapshot, error: Option<String>) -> Value {
    json!({"catalog_included":s.plans.len(),"prepared_as_of":s.prepared_as_of,"license":"VPS Price Index CC-BY 4.0; Cloud FinOps and Aéza public price observations, source links on offers", "limitations":s.limitations,"fx":s.fx,"providers":s.providers,"raw_product_count":s.raw_product_count,"official_aeza_count":s.aeza_plans.len(),"official_aeza_available":s.aeza_plans.iter().filter(|p|p.details.as_ref().is_some_and(|d|d.availability!="unavailable")).count(),"snapshot_status":error.unwrap_or_else(||"verified_runtime_snapshot".into()),"catalog_version":s.catalog_version})
}
impl ApiState {
    fn resolved(mut self) -> Self {
        if let Some(store) = &self.store {
            let (s, e) = store.current();
            self.sources = Arc::new(sources(&s, e));
            self.market = Arc::new(s.plans);
        }
        self
    }
}
/// Embed the same handlers with a controlled immutable catalog and explicit provenance.
/// HTTP callers cannot replace the catalog.
pub fn router_with_catalog(market: Vec<crate::Plan>) -> Router {
    let sources = json!({
        "catalog_included": market.len(),
        "catalog_sha256": crate::hash(&serde_json::to_vec(&market).unwrap()),
        "source_note": "Caller-supplied catalog; inspect source/origin/observed_at on each plan. Bundled VPS Price Index manifest does not apply.",
        "limitations": ["Controlled or embedded catalog: source validity is the caller's responsibility."]
    });
    build_router(market, sources, None)
}
fn build_router(
    market: Vec<crate::Plan>,
    sources: Value,
    store: Option<Arc<crate::market_data::Store>>,
) -> Router {
    Router::new()
        .route(
            "/api/market/v1/health",
            get(|| async { Json(json!({"status":"ok","engine":crate::VERSION})) }),
        )
        .route(
            "/api/market/v1/catalog",
            get(|State(state): State<ApiState>| async move { Json((*state.resolved().market).clone()) }),
        )
        .route(
            "/api/market/v1/sources",
            get(|State(state): State<ApiState>| async move { Json((*state.resolved().sources).clone()) }),
        )
        .route(
            "/api/market/v1/example",
            get(|| async { Json(crate::example()) }),
        )
        .route(
            "/api/market/v1/openapi.json",
            get(|| async {
                Json(serde_json::from_str::<Value>(include_str!("../docs/openapi.json")).unwrap())
            }),
        )
        .route("/api/market/v1/aeza-plans",get(|State(state):State<ApiState>|async move {Json(state.store.as_ref().map(|s|s.current().0.aeza_plans).unwrap_or_default())}))
        .route("/api/market/v1/market-data",get(|State(state):State<ApiState>|async move {Json(state.store.as_ref().map(|s|serde_json::to_value(s.current().0).unwrap()).unwrap_or_else(||json!({"error":"injected catalog has no runtime snapshot"})))}))
        .route("/api/market/v2/health",get(||async {Json(json!({"status":"ok","engine":crate::VERSION,"currency":"RUB"}))}))
        .route("/api/market/v2/catalog",get(|State(state):State<ApiState>|async move {let state=rub_state(state);Json(crate::currency::wire(serde_json::to_value(&*state.market).unwrap(),true))}))
        .route("/api/market/v2/aeza-plans",get(|State(state):State<ApiState>|async move {let state=state.resolved();let fx=state_fx(&state);let plans=state.store.as_ref().map(|s|s.current().0.aeza_plans).unwrap_or_default().into_iter().map(|p|crate::currency::plan_to_rub(p,&fx)).collect::<Vec<_>>();Json(crate::currency::wire(serde_json::to_value(plans).unwrap(),true))}))
        .route("/api/market/v2/example",get(|State(state):State<ApiState>|async move {let state=state.resolved();let fx=state_fx(&state);Json(crate::currency::wire(serde_json::to_value(rub_example(&state,&fx)).unwrap(),true))}))
        .route("/api/market/v2/sources",get(|State(state):State<ApiState>|async move {let mut v=(*state.resolved().sources).clone();v["currency"]=json!("RUB");Json(v)}))
        .route("/api/market/v2/market-data",get(|State(state):State<ApiState>|async move {Json(state.store.as_ref().map(|s|serde_json::to_value(s.current().0).unwrap()).unwrap_or_else(||json!({"error":"no snapshot"})))}))
        .route("/api/market/v2/contexts",get(|State(state):State<ApiState>|async move {Json(state.store.as_ref().map(|s|s.current().0.contexts).unwrap_or_else(||json!({})))}))
        .route("/api/market/v2/simulate",post(run_rub))
        .route("/api/market/v2/competition",post(competition_rub))
        .route("/api/market/v2/behavior",post(behavior_rub))
        .route("/api/market/v2/statistics",post(statistics_rub))
        .route("/api/market/v1/simulate", post(run))
        .route("/api/market/v1/competition", post(competition))
        .layer(DefaultBodyLimit::max(128 * 1024))
        .with_state(ApiState {
            slots: Arc::new(Semaphore::new(2)),
            statistics_slots: Arc::new(Semaphore::new(1)),
            market: Arc::new(market),
            store,
            sources: Arc::new(sources),
        })
}
async fn run(
    State(state): State<ApiState>,
    payload: Result<Json<Scenario>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let state = state.resolved();
    let mut s = match payload {
        Ok(Json(s)) => s,
        Err(e) => return (e.status(), Json(json!({"error":e.body_text()}))),
    };
    let trusted_fx = state_fx(&state);
    if s.valuation_fx.as_ref().is_some_and(|fx| {
        serde_json::to_value(fx).unwrap() != serde_json::to_value(&trusted_fx).unwrap()
    }) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(
                json!({"error":"FX does not match verified dataset; reload the current example/sources or restore the matching snapshot for replay"}),
            ),
        );
    }
    if s.currency == "RUB" {
        s.valuation_fx = Some(trusted_fx);
    }
    if let Err(e) = s.validate(&state.market) {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":e})));
    }
    let permit = match state.slots.try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"simulation capacity reached"})),
            );
        }
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        simulate(s, (*state.market).clone())
    })
    .await
    {
        Ok(Ok(report)) => (StatusCode::OK, Json(serde_json::to_value(report).unwrap())),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":e}))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"simulation worker failed"})),
        ),
    }
}

async fn competition(
    State(state): State<ApiState>,
    payload: Result<Json<crate::competition::Request>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let state = state.resolved();
    let mut request = match payload {
        Ok(Json(r)) => r,
        Err(e) => return (e.status(), Json(json!({"error":e.body_text()}))),
    };
    let trusted_fx = state_fx(&state);
    if request.scenario.valuation_fx.as_ref().is_some_and(|fx| {
        serde_json::to_value(fx).unwrap() != serde_json::to_value(&trusted_fx).unwrap()
    }) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"FX does not match verified dataset"})),
        );
    }
    if request.scenario.currency == "RUB" {
        request.scenario.valuation_fx = Some(trusted_fx);
    }
    if let Err(e) = request.scenario.validate(&state.market) {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":e})));
    }
    let permit = match state.slots.try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"simulation capacity reached"})),
            );
        }
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::competition::analyze(request, (*state.market).clone())
    })
    .await
    {
        Ok(Ok(result)) => (StatusCode::OK, Json(serde_json::to_value(result).unwrap())),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":e}))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"simulation worker failed"})),
        ),
    }
}

fn state_fx(state: &ApiState) -> crate::market_data::Fx {
    serde_json::from_value(state.sources["fx"].clone())
        .unwrap_or_else(|_| crate::market_data::bundled_fx())
}
fn rub_state(state: ApiState) -> ApiState {
    let mut state = state.resolved();
    let fx = state_fx(&state);
    state.market = Arc::new(
        state
            .market
            .iter()
            .cloned()
            .map(|p| crate::currency::plan_to_rub(p, &fx))
            .collect(),
    );
    state.store = None;
    state
}
async fn run_rub(
    State(state): State<ApiState>,
    payload: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let v = match payload {
        Ok(Json(v)) => v,
        Err(e) => return (e.status(), Json(json!({"error":e.body_text()}))),
    };
    if !v.is_object() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"object required"})),
        );
    }
    let mut v = crate::currency::wire(v, false);
    v["currency"] = json!("RUB");
    let scenario = match serde_json::from_value(v) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error":e.to_string()})),
            );
        }
    };
    let (status, Json(v)) = run(State(rub_state(state)), Ok(Json(scenario))).await;
    (status, Json(crate::currency::wire(v, true)))
}
async fn competition_rub(
    State(state): State<ApiState>,
    payload: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let v = match payload {
        Ok(Json(v)) => v,
        Err(e) => return (e.status(), Json(json!({"error":e.body_text()}))),
    };
    if !v.is_object() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"object required"})),
        );
    }
    let mut v = crate::currency::wire(v, false);
    if !v["scenario"].is_object() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"scenario required"})),
        );
    }
    v["scenario"]["currency"] = json!("RUB");
    let request = match serde_json::from_value(v) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error":e.to_string()})),
            );
        }
    };
    let (status, Json(v)) = competition(State(rub_state(state)), Ok(Json(request))).await;
    (status, Json(crate::currency::wire(v, true)))
}

fn rub_example(state: &ApiState, fx: &crate::market_data::Fx) -> Scenario {
    let mut s = crate::currency::scenario_to_rub(crate::example(), fx);
    if let Some(store) = &state.store {
        let snapshot = store.current().0;
        let mut available: Vec<_> = snapshot
            .aeza_plans
            .into_iter()
            .filter(|p| {
                p.details.as_ref().is_some_and(|d| {
                    d.availability == "available_from_source"
                        && d.cpu_class.as_deref() == Some("shared")
                })
            })
            .collect();
        available.sort_by(|a, b| {
            (a.regions.first().map(|r| r != "Moscow"), &a.regions)
                .cmp(&(b.regions.first().map(|r| r != "Moscow"), &b.regions))
                .then_with(|| a.vcpu.total_cmp(&b.vcpu))
        });
        if let Some(first) = available.first() {
            let region = first.regions[0].clone();
            s.market = first.details.as_ref().unwrap().market.clone();
            s.aeza_plans = available
                .into_iter()
                .filter(|p| p.regions == [region.clone()])
                .take(3)
                .map(|p| crate::currency::plan_to_rub(p, fx))
                .collect();
            for g in &mut s.segments {
                g.region = Some(region.clone());
            }
        }
        if let Some(weights) =
            snapshot.contexts["akash"]["monthly_profile"]["normalized_weights"].as_array()
        {
            if weights.len() == s.months {
                if let Some(v) = weights
                    .iter()
                    .map(|x| x.as_f64())
                    .collect::<Option<Vec<_>>>()
                {
                    s.arrival_weights = v;
                }
            }
        }
    }
    s
}

async fn behavior_rub(
    State(state): State<ApiState>,
    payload: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let v = match payload {
        Ok(Json(v)) => v,
        Err(e) => return (e.status(), Json(json!({"error":e.body_text()}))),
    };
    let mut v = crate::currency::wire(v, false);
    if !v["scenario"].is_object() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"scenario object required"})),
        );
    }
    v["scenario"]["currency"] = json!("RUB");
    let request = match serde_json::from_value(v) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error":e.to_string()})),
            );
        }
    };
    let scenario = match crate::behavior::prepare(request) {
        Ok(s) => s,
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":e}))),
    };
    let (status, Json(v)) = run(State(rub_state(state)), Ok(Json(scenario))).await;
    if status != StatusCode::OK {
        return (status, Json(crate::currency::wire(v, true)));
    }
    let report = serde_json::from_value(v).expect("engine report schema");
    (
        status,
        Json(crate::currency::wire(
            crate::behavior::analyze_report(report),
            true,
        )),
    )
}

async fn statistics_rub(
    State(state): State<ApiState>,
    payload: Result<Json<Value>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let v = match payload {
        Ok(Json(v)) => v,
        Err(e) => return (e.status(), Json(json!({"error":e.body_text()}))),
    };
    let mut v = crate::currency::wire(v, false);
    if !v["scenario"].is_object() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"scenario object required"})),
        );
    }
    v["scenario"]["currency"] = json!("RUB");
    let mut request: crate::statistics::Request = match serde_json::from_value(v) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error":e.to_string()})),
            );
        }
    };
    let state = rub_state(state);
    let fx = state_fx(&state);
    if request
        .scenario
        .valuation_fx
        .as_ref()
        .is_some_and(|f| serde_json::to_value(f).unwrap() != serde_json::to_value(&fx).unwrap())
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"FX does not match verified dataset"})),
        );
    }
    request.scenario.valuation_fx = Some(fx);
    let scenario = match crate::statistics::prepare(request, &state.market) {
        Ok(s) => s,
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":e}))),
    };
    let batch = match state.statistics_slots.try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"statistics batch already running"})),
            );
        }
    };
    let worker = match state.slots.try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({"error":"simulation capacity reached"})),
            );
        }
    };
    match tokio::task::spawn_blocking(move || {
        let _batch = batch;
        let _worker = worker;
        crate::statistics::analyze(scenario, (*state.market).clone())
    })
    .await
    {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(crate::currency::wire(
                serde_json::to_value(result).unwrap(),
                true,
            )),
        ),
        Ok(Err(e)) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error":e}))),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"statistics worker failed"})),
        ),
    }
}

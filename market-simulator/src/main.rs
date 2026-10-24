#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "--check-data") {
        let path = args.get(2).ok_or("snapshot path required")?;
        let s = aeza_market_simulator::market_data::load(std::path::Path::new(path))?;
        println!(
            "verified: {} competitor offers, {} Aéza plans",
            s.plans.len(),
            s.aeza_plans.len()
        );
        return Ok(());
    }

    if args.get(1).is_some_and(|a| a == "--scenario") {
        let path = args
            .get(2)
            .ok_or("usage: --scenario scenario.json [catalog.json]")?;
        let scenario = serde_json::from_slice(&std::fs::read(path)?)?;
        let market = if let Some(path) = args.get(3) {
            serde_json::from_slice(&std::fs::read(path)?)?
        } else {
            aeza_market_simulator::market_data::runtime().plans
        };
        let report = aeza_market_simulator::simulate(scenario, market)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let addr = std::env::var("MARKET_BIND").unwrap_or_else(|_| "127.0.0.1:8091".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("Market Simulator: http://{addr}/api/market/v1/health");
    axum::serve(listener, aeza_market_simulator::api::router()).await?;
    Ok(())
}

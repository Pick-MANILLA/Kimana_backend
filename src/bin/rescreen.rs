//! Manual rescreening job (ISSUE-02): re-runs KYB checks against every
//! approved customer and updates their stored risk rating and per-transfer
//! limit. `cargo run --bin rescreen` until a real scheduler lands.

use kimana_backend::domain::onboarding::service::rescreen_approved_customers;
use kimana_backend::{config::Config, db, state::AppState};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await?;
    let state = AppState {
        pool,
        config: Arc::new(config),
    };

    let outcomes = rescreen_approved_customers(&state).await?;
    for outcome in &outcomes {
        println!(
            "customer {} (application {}): {} risk, limit {:?} minor",
            outcome.customer_id,
            outcome.application_id,
            outcome.risk_rating,
            outcome.transaction_limit_minor
        );
    }
    println!("rescreened {} approved customer(s)", outcomes.len());
    Ok(())
}

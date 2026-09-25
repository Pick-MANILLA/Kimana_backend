//! Links a customer to a Bridge USD virtual account (issue #79), once they
//! have passed Bridge's own KYB:
//! `cargo run --bin link-bridge -- <kimana-customer-id> <bridge-customer-id>`.
//! A CLI rather than a route because there is no operator role yet.

use kimana_backend::domain::collections::link_bridge_account;
use kimana_backend::{config::Config, db, state::AppState};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [customer_id, bridge_customer_id] = args.as_slice() else {
        anyhow::bail!("usage: link-bridge <kimana-customer-id> <bridge-customer-id>");
    };
    let customer_id: Uuid = customer_id.parse()?;

    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await?;
    let state = AppState::new(pool, config);

    let account = link_bridge_account(&state, customer_id, bridge_customer_id)
        .await
        .map_err(|err| anyhow::anyhow!("{err} (details in the log above)"))?;
    println!("{}", serde_json::to_string_pretty(&account)?);
    Ok(())
}

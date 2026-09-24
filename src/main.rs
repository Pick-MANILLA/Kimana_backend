use kimana_backend::settlement::{ListenerConfig, SettlementConfig, SettlementListener};
use kimana_backend::{build_app, config::Config, db, state::AppState};
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await?;
    db::run_migrations(&pool).await?;

    if let Some(settlement) = SettlementConfig::from_env() {
        let settlement = settlement.map_err(anyhow::Error::msg)?;
        let listener = SettlementListener::new(
            pool.clone(),
            settlement.rpc_url,
            ListenerConfig {
                vault: settlement.vault,
                confirmations: config.settlement_confirmations,
                start_block: config.settlement_start_block,
                poll: Duration::from_millis(config.settlement_poll_ms),
                max_range: config.settlement_log_range,
            },
        );
        tracing::info!(vault = %settlement.vault, "settlement listener started");
        tokio::spawn(listener.run());
    }

    let addr = format!("{}:{}", config.host, config.port);
    let state = AppState::new(pool, config);
    let app = build_app(state);

    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("kimana-backend listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

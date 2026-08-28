use kimana_backend::{build_app, config::Config, db, state::AppState};
use std::sync::Arc;
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

    let addr = format!("{}:{}", config.host, config.port);
    let state = AppState {
        pool,
        config: Arc::new(config),
    };
    let app = build_app(state);

    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("kimana-backend listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

use kimana_backend::{config::Config, db, seed};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env();
    let pool = db::connect(&config.database_url).await?;
    db::run_migrations(&pool).await?;
    seed::seed(&pool).await?;
    println!("seed complete");
    Ok(())
}

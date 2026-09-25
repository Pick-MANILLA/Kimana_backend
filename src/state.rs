use crate::config::Config;
use crate::domain::fx::FxResilience;
use crate::partners::Partners;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub fx_resilience: Arc<FxResilience>,
    pub partners: Arc<Partners>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        let fx_resilience = Arc::new(FxResilience::from_config(&config));
        let partners = Arc::new(Partners::from_config(&config));
        AppState {
            pool,
            config: Arc::new(config),
            fx_resilience,
            partners,
        }
    }
}

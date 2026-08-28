use crate::error::ApiError;
use crate::ids::DEMO_USER_ID;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

/// Resolved request session plus the customer id (the contract's Session omits
/// it). P1 slice: a single seeded demo customer, looked up by DEMO_USER_ID.
/// Real login is P1-later — only this extractor changes.
#[derive(Debug, Clone)]
pub struct Session {
    pub user_id: Uuid,
    pub role: String,
    pub display_name: String,
    pub operator_permissions: Vec<String>,
    pub customer_id: Uuid,
}

impl Session {
    pub fn role_str(&self) -> &str {
        &self.role
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    user_id: Uuid,
    role: String,
    display_name: String,
    operator_permissions: Vec<String>,
    customer_id: Uuid,
}

impl FromRequestParts<AppState> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let row: Option<SessionRow> = sqlx::query_as(
            "select u.id as user_id, u.role, u.display_name,
                    u.operator_permissions, c.id as customer_id
               from users u
               join customers c on c.primary_user_id = u.id
              where u.id = $1",
        )
        .bind(DEMO_USER_ID)
        .fetch_optional(&state.pool)
        .await?;

        let Some(row) = row else {
            return Err(ApiError::unauthorized(
                "Demo session is not seeded. Run `cargo run --bin seed`.",
            ));
        };

        Ok(Session {
            user_id: row.user_id,
            role: row.role,
            display_name: row.display_name,
            operator_permissions: row.operator_permissions,
            customer_id: row.customer_id,
        })
    }
}

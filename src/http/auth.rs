use crate::error::ApiError;
use crate::state::AppState;
use crate::util::hash_token;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::cookie::CookieJar;
use uuid::Uuid;

/// Name of the cookie holding the raw (unhashed) session token. Shared with
/// `domain::auth::routes`, which sets/clears it on register/login/logout.
pub const SESSION_COOKIE_NAME: &str = "kimana_session";

/// Resolved request session plus the customer id (the contract's Session omits
/// it). Backed by a real, DB-stored session: the `Session` extractor reads
/// the `kimana_session` cookie, hashes it, and looks up a live row in
/// `sessions` (not expired, not revoked).
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
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let unauthorized = || ApiError::unauthorized("Sign in to continue.");

        let jar = CookieJar::from_request_parts(parts, state)
            .await
            .expect("CookieJar extraction is infallible");
        let token = jar
            .get(SESSION_COOKIE_NAME)
            .map(|c| c.value().to_string())
            .ok_or_else(unauthorized)?;
        let token_hash = hash_token(&token);

        let row: Option<SessionRow> = sqlx::query_as(
            "select u.id as user_id, u.role, u.display_name,
                    u.operator_permissions, c.id as customer_id
               from sessions s
               join users u on u.id = s.user_id
               join customers c on c.primary_user_id = u.id
              where s.token_hash = $1 and s.revoked_at is null and s.expires_at > now()",
        )
        .bind(&token_hash)
        .fetch_optional(&state.pool)
        .await?;

        let Some(row) = row else {
            return Err(unauthorized());
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

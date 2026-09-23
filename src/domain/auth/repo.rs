use crate::error::ApiResult;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct UserAuthRow {
    pub id: Uuid,
    pub display_name: String,
    pub password_hash: Option<String>,
}

pub async fn find_user_by_email(pool: &PgPool, email: &str) -> ApiResult<Option<UserAuthRow>> {
    let row = sqlx::query_as(
        "select id, display_name, password_hash from users where lower(email) = lower($1)",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn insert_user(
    conn: &mut PgConnection,
    id: Uuid,
    display_name: &str,
    email: &str,
    password_hash: &str,
) -> ApiResult<()> {
    sqlx::query(
        "insert into users (id, role, display_name, operator_permissions, email, password_hash)
         values ($1, 'customer', $2, '{}', $3, $4)",
    )
    .bind(id)
    .bind(display_name)
    .bind(email)
    .bind(password_hash)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn insert_customer(
    conn: &mut PgConnection,
    id: Uuid,
    legal_name: &str,
    primary_user_id: Uuid,
) -> ApiResult<()> {
    sqlx::query("insert into customers (id, legal_name, primary_user_id) values ($1, $2, $3)")
        .bind(id)
        .bind(legal_name)
        .bind(primary_user_id)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn insert_draft_application(
    conn: &mut PgConnection,
    id: Uuid,
    customer_id: Uuid,
) -> ApiResult<()> {
    sqlx::query(
        "insert into onboarding_applications (id, customer_id, status) values ($1, $2, 'draft')",
    )
    .bind(id)
    .bind(customer_id)
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
pub struct SessionContextRow {
    pub role: String,
    pub display_name: String,
    pub operator_permissions: Vec<String>,
    pub customer_id: Uuid,
}

/// The same `users join customers` lookup the `Session` extractor performs,
/// reused here to build a `Session` right after a successful login.
pub async fn find_session_context(
    pool: &PgPool,
    user_id: Uuid,
) -> ApiResult<Option<SessionContextRow>> {
    let row = sqlx::query_as(
        "select u.role, u.display_name, u.operator_permissions, c.id as customer_id
           from users u
           join customers c on c.primary_user_id = u.id
          where u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn insert_session(
    conn: &mut PgConnection,
    id: Uuid,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> ApiResult<()> {
    sqlx::query(
        "insert into sessions (id, user_id, token_hash, expires_at) values ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(conn)
    .await?;
    Ok(())
}

/// Idempotent: revoking an already-revoked or unknown token is not an error.
pub async fn revoke_session(pool: &PgPool, token_hash: &str) -> ApiResult<()> {
    sqlx::query(
        "update sessions set revoked_at = now() where token_hash = $1 and revoked_at is null",
    )
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

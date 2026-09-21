use super::repo;
use super::schema;
use crate::audit::{write_audit, AuditEntry};
use crate::error::{ApiError, ApiResult};
use crate::http::Session;
use crate::state::AppState;
use crate::util;
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

pub(crate) const SESSION_TTL_DAYS: i64 = 30;

pub struct RegisterInput {
    pub email: String,
    pub password: String,
    pub display_name: String,
    pub legal_name: String,
}

pub struct Authenticated {
    pub session: Session,
    pub raw_token: String,
}

pub async fn register(state: &AppState, input: RegisterInput) -> ApiResult<Authenticated> {
    schema::validate_register(
        &input.email,
        &input.password,
        &input.display_name,
        &input.legal_name,
    )?;

    if repo::find_user_by_email(&state.pool, &input.email)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict(
            "An account with that email already exists.",
        ));
    }

    let password_hash = util::hash_password(&input.password)?;

    let user_id = Uuid::new_v4();
    let customer_id = Uuid::new_v4();
    let application_id = Uuid::new_v4();

    let mut tx = state.pool.begin().await?;

    repo::insert_user(
        &mut tx,
        user_id,
        &input.display_name,
        &input.email,
        &password_hash,
    )
    .await?;
    repo::insert_customer(&mut tx, customer_id, &input.legal_name, user_id).await?;
    repo::insert_draft_application(&mut tx, application_id, customer_id).await?;

    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(user_id),
            actor_role: Some("customer"),
            action: "auth.user_registered",
            entity_type: "user",
            entity_id: user_id.to_string(),
            before: None,
            after: Some(json!({
                "email": input.email,
                "displayName": input.display_name,
                "legalName": input.legal_name,
            })),
        },
    )
    .await?;

    let raw_token = util::generate_session_token();
    let token_hash = util::hash_token(&raw_token);
    repo::insert_session(
        &mut tx,
        Uuid::new_v4(),
        user_id,
        &token_hash,
        Utc::now() + Duration::days(SESSION_TTL_DAYS),
    )
    .await?;

    tx.commit().await?;

    Ok(Authenticated {
        session: Session {
            user_id,
            role: "customer".to_string(),
            display_name: input.display_name,
            operator_permissions: Vec::new(),
            customer_id,
        },
        raw_token,
    })
}

pub async fn login(state: &AppState, email: &str, password: &str) -> ApiResult<Authenticated> {
    let invalid = || ApiError::unauthorized("Invalid email or password.");

    let user = repo::find_user_by_email(&state.pool, email)
        .await?
        .ok_or_else(invalid)?;
    let hash = user.password_hash.as_deref().ok_or_else(invalid)?;
    if !util::verify_password(password, hash) {
        return Err(invalid());
    }

    let ctx = repo::find_session_context(&state.pool, user.id)
        .await?
        .ok_or_else(invalid)?;

    let raw_token = util::generate_session_token();
    let token_hash = util::hash_token(&raw_token);

    let mut tx = state.pool.begin().await?;
    repo::insert_session(
        &mut tx,
        Uuid::new_v4(),
        user.id,
        &token_hash,
        Utc::now() + Duration::days(SESSION_TTL_DAYS),
    )
    .await?;
    tx.commit().await?;

    Ok(Authenticated {
        session: Session {
            user_id: user.id,
            role: ctx.role,
            display_name: ctx.display_name,
            operator_permissions: ctx.operator_permissions,
            customer_id: ctx.customer_id,
        },
        raw_token,
    })
}

pub async fn logout(state: &AppState, raw_token: &str) -> ApiResult<()> {
    repo::revoke_session(&state.pool, &util::hash_token(raw_token)).await
}

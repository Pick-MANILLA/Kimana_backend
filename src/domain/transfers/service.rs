use super::engine;
use super::repo::{self, InsertTransfer};
use crate::audit::{write_audit, AuditEntry};
use crate::contract::transfer::{Transfer, TransferStatus, TransferTimeline};
use crate::domain::{quote, recipients};
use crate::error::{ApiError, ApiResult};
use crate::http::Session;
use crate::state::AppState;
use crate::util::tagged_reference;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

pub struct CreateTransferInput {
    pub idempotency_key: String,
    pub quote_id: String,
    pub recipient_id: String,
}

async fn owned_transfer(state: &AppState, session: &Session, id: &str) -> ApiResult<Transfer> {
    repo::find_by_id(&state.pool, id)
        .await?
        .filter(|t| t.customer_id == session.customer_id.to_string())
        .ok_or_else(|| ApiError::not_found("That transfer couldn't be found."))
}

pub async fn create_transfer(
    state: &AppState,
    session: &Session,
    input: CreateTransferInput,
) -> ApiResult<Transfer> {
    if input.idempotency_key.trim().len() < 8 {
        return Err(ApiError::validation("Provide a stable idempotency key."));
    }

    if let Some(existing) =
        repo::find_by_idempotency_key(&state.pool, session.customer_id, &input.idempotency_key)
            .await?
    {
        return Ok(existing);
    }

    let quote = quote::find_by_id(&state.pool, &input.quote_id)
        .await?
        .filter(|q| q.customer_id == session.customer_id)
        .ok_or_else(|| ApiError::not_found("That quote couldn't be found."))?;
    if quote.expires_at <= Utc::now() {
        return Err(ApiError::rate_expired(
            "That quote has expired. Request a fresh one.",
        ));
    }

    let recipient = recipients::find_by_id(&state.pool, &input.recipient_id, session.customer_id)
        .await?
        .ok_or_else(|| ApiError::not_found("That recipient couldn't be found."))?;
    let recipient_uuid = Uuid::parse_str(&recipient.id).unwrap();

    let fq = &quote.firm_quote;
    let mut tx = state.pool.begin().await?;
    let inserted = repo::insert(
        &mut tx,
        InsertTransfer {
            reference: &tagged_reference("KM"),
            customer_id: session.customer_id,
            idempotency_key: &input.idempotency_key,
            recipient_id: recipient_uuid,
            send_currency: fq.send_currency,
            receive_currency: fq.receive_currency,
            send_amount_minor: fq.breakdown.send_amount.amount_minor,
            receive_amount_minor: fq.breakdown.receive_amount.amount_minor,
            trade_description: None,
            quote_snapshot: fq,
            initial_status: TransferStatus::Created,
        },
    )
    .await?;

    let Some(transfer_id) = inserted else {
        tx.rollback().await?;
        if let Some(raced) =
            repo::find_by_idempotency_key(&state.pool, session.customer_id, &input.idempotency_key)
                .await?
        {
            return Ok(raced);
        }
        return Err(ApiError::new(
            crate::error::ErrorCode::Conflict,
            "Could not create the transfer. Try again.",
        )
        .retryable(true));
    };

    repo::append_history(&mut tx, transfer_id, TransferStatus::Created, None, None).await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "transfer.created",
            entity_type: "transfer",
            entity_id: transfer_id.to_string(),
            before: None,
            after: Some(json!({
                "status": "CREATED",
                "quoteId": quote.id.to_string(),
                "recipientId": recipient.id,
            })),
        },
    )
    .await?;
    tx.commit().await?;

    engine::advance_to_awaiting_funds(state, transfer_id).await?;
    schedule_simulated_progression(state, transfer_id);

    repo::find_by_id_uuid(&state.pool, transfer_id)
        .await?
        .ok_or_else(ApiError::server_error)
}

/// Stands in for a collection-partner "funds received" webhook plus the
/// settlement/payout pipeline. Negative auto-advance disables it (tests drive
/// the engine directly). A restart between AWAITING_FUNDS and COMPLETED leaves
/// the transfer parked — no resume sweep.
fn schedule_simulated_progression(state: &AppState, transfer_id: Uuid) {
    let auto_ms = state.config.transfer_auto_advance_ms;
    if auto_ms < 0 {
        return;
    }
    let state = state.clone();
    let step_ms = state.config.transfer_step_delay_ms;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(auto_ms as u64)).await;
        if let Err(err) = engine::advance_to_completion(&state, transfer_id, step_ms).await {
            tracing::warn!(?err, %transfer_id, "simulated progression failed");
        }
    });
}

pub async fn get_transfer(state: &AppState, session: &Session, id: &str) -> ApiResult<Transfer> {
    owned_transfer(state, session, id).await
}

pub async fn get_timeline(
    state: &AppState,
    session: &Session,
    id: &str,
) -> ApiResult<TransferTimeline> {
    let owner = repo::get_owner_and_status(&state.pool, id)
        .await?
        .filter(|o| o.customer_id == session.customer_id)
        .ok_or_else(|| ApiError::not_found("That transfer couldn't be found."))?;
    let history = repo::get_history(&state.pool, Uuid::parse_str(id).unwrap()).await?;
    Ok(TransferTimeline {
        transfer_id: id.to_string(),
        history,
        is_terminal: owner.current_status.is_terminal(),
    })
}

pub async fn list_transfers(
    state: &AppState,
    session: &Session,
    status: Option<TransferStatus>,
) -> ApiResult<Vec<Transfer>> {
    repo::list_by_customer(&state.pool, session.customer_id, status).await
}

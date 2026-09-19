//! Transfer state-machine engine. One transition per transaction (history row +
//! audit + ledger postings together).
//!
//! Ledger model — customer-account-centric FX-through payment:
//!
//! - `FUNDED`: `-sendAmount` from the send-currency account
//! - `SETTLED`: `+receiveAmount` to the receive-currency account
//! - `COMPLETED`: `-receiveAmount` from the receive-currency account (to beneficiary)
//! - `REVERSED`: `+sendAmount` back to the send account, linked
//!
//! Net effect on a completed transfer: `-sendAmount` in the send currency.

use super::repo;
use super::state_machine::{assert_transition, forward_step};
use crate::audit::{write_audit, AuditEntry};
use crate::contract::common::CurrencyCode;
use crate::contract::transfer::TransferStatus::{self, *};
use crate::domain::ledger::{
    account_balance_minor, get_or_create_account, post_ledger_entry, LedgerPosting,
};
use crate::domain::screening;
use crate::error::{ApiError, ApiResult, ErrorCode};
use crate::state::AppState;
use crate::util::{iso, tagged_reference};
use chrono::Utc;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct LockedTransfer {
    id: Uuid,
    reference: String,
    customer_id: Uuid,
    recipient_id: Uuid,
    recipient_country: String,
    send_currency: String,
    receive_currency: String,
    send_amount_minor: i64,
    receive_amount_minor: i64,
    current_status: String,
    /// Payload of the most recently appended history row — when
    /// `current_status` is `SCREENED`, this is that screening outcome.
    latest_payload: Option<Value>,
}

async fn lock(conn: &mut sqlx::PgConnection, id: Uuid) -> ApiResult<LockedTransfer> {
    sqlx::query_as::<_, LockedTransfer>(
        "select t.id, t.reference, t.customer_id, t.recipient_id, r.country as recipient_country,
                t.send_currency, t.receive_currency,
                t.send_amount_minor, t.receive_amount_minor, t.current_status,
                h.payload as latest_payload
           from transfers t
           join recipients r on r.id = t.recipient_id
           left join lateral (
             select payload from transfer_state_history
              where transfer_id = t.id order by position desc limit 1
           ) h on true
          where t.id = $1
          for update of t",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| ApiError::not_found("Transfer not found."))
}

/// True only when `t.current_status` is `SCREENED` and that screening
/// outcome held the transfer for review.
fn is_held(t: &LockedTransfer) -> bool {
    t.latest_payload
        .as_ref()
        .and_then(|p| p.get("hold"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn payload_for(
    t: &LockedTransfer,
    to: TransferStatus,
    amount_threshold_minor: i64,
) -> ApiResult<Option<Value>> {
    Ok(match to {
        Screened => {
            let outcome = screening::screen(
                screening::ScreeningInput {
                    recipient_country: &t.recipient_country,
                    send_amount_minor: t.send_amount_minor,
                    send_currency: CurrencyCode::parse(&t.send_currency)?,
                },
                amount_threshold_minor,
            );
            let mut payload = json!({ "hold": outcome.hold });
            if outcome.hold {
                payload["holdReason"] = json!(outcome.hold_reason);
                payload["expectedResolutionBy"] =
                    json!(iso(Utc::now() + chrono::Duration::hours(48)));
            }
            Some(payload)
        }
        AwaitingFunds => Some(json!({ "fundingReference": tagged_reference("FR") })),
        Completed => Some(json!({ "payoutReference": tagged_reference("PO") })),
        _ => None,
    })
}

async fn post_ledger_for(
    conn: &mut sqlx::PgConnection,
    t: &LockedTransfer,
    to: TransferStatus,
) -> ApiResult<()> {
    let send_ccy = CurrencyCode::parse(&t.send_currency)?;
    let recv_ccy = CurrencyCode::parse(&t.receive_currency)?;

    match to {
        Funded => {
            let acct = get_or_create_account(&mut *conn, t.customer_id, send_ccy).await?;
            post_ledger_entry(
                &mut *conn,
                LedgerPosting {
                    account_id: acct,
                    transfer_id: t.id,
                    amount_minor: -t.send_amount_minor,
                    currency: send_ccy,
                    description: &format!("Transfer {} — funded", t.reference),
                    reversal_of_entry_id: None,
                },
            )
            .await?;
        }
        Settled => {
            let acct = get_or_create_account(&mut *conn, t.customer_id, recv_ccy).await?;
            post_ledger_entry(
                &mut *conn,
                LedgerPosting {
                    account_id: acct,
                    transfer_id: t.id,
                    amount_minor: t.receive_amount_minor,
                    currency: recv_ccy,
                    description: &format!("Transfer {} — converted", t.reference),
                    reversal_of_entry_id: None,
                },
            )
            .await?;
        }
        Completed => {
            let acct = get_or_create_account(&mut *conn, t.customer_id, recv_ccy).await?;
            let beneficiary: Option<String> =
                sqlx::query_scalar("select account_name from recipients where id = $1")
                    .bind(t.recipient_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            let beneficiary = beneficiary.unwrap_or_else(|| "beneficiary".into());
            post_ledger_entry(
                &mut *conn,
                LedgerPosting {
                    account_id: acct,
                    transfer_id: t.id,
                    amount_minor: -t.receive_amount_minor,
                    currency: recv_ccy,
                    description: &format!("Transfer {} — paid to {beneficiary}", t.reference),
                    reversal_of_entry_id: None,
                },
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn apply(
    conn: &mut sqlx::PgConnection,
    t: &LockedTransfer,
    actor_id: Option<Uuid>,
    from: TransferStatus,
    to: TransferStatus,
    payload: Option<Value>,
) -> ApiResult<()> {
    assert_transition(from, to)?;
    repo::append_history(&mut *conn, t.id, to, payload.as_ref(), None).await?;
    repo::set_status(&mut *conn, t.id, to).await?;
    post_ledger_for(&mut *conn, t, to).await?;

    let mut after = json!({ "status": to.as_str() });
    if let Some(Value::Object(fields)) = &payload {
        if let Some(obj) = after.as_object_mut() {
            for (k, v) in fields {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    write_audit(
        &mut *conn,
        AuditEntry {
            actor_id,
            actor_role: actor_id.map(|_| "customer"),
            action: "transfer.state_change",
            entity_type: "transfer",
            entity_id: t.id.to_string(),
            before: Some(json!({ "status": from.as_str() })),
            after: Some(after),
        },
    )
    .await?;
    Ok(())
}

/// Applies exactly one transition. Returns the resulting status (unchanged if blocked).
pub async fn advance_once(
    state: &AppState,
    transfer_id: Uuid,
    actor_id: Option<Uuid>,
) -> ApiResult<TransferStatus> {
    let mut tx = state.pool.begin().await?;
    let t = lock(&mut tx, transfer_id).await?;
    let from = TransferStatus::parse(&t.current_status)?;

    if from == Screened && is_held(&t) {
        return Err(ApiError::compliance_hold(
            "This transfer is on hold pending compliance review.",
        ));
    }

    let Some(to) = forward_step(from) else {
        tx.commit().await?;
        return Ok(from);
    };

    if from == AwaitingFunds {
        let send_ccy = CurrencyCode::parse(&t.send_currency)?;
        let acct = get_or_create_account(&mut tx, t.customer_id, send_ccy).await?;
        let balance = account_balance_minor(&mut tx, acct).await?;
        if balance < t.send_amount_minor {
            apply(
                &mut tx,
                &t,
                actor_id,
                from,
                Rejected,
                Some(
                    json!({ "failureCategory": "validation", "reasonCode": "INSUFFICIENT_FUNDS" }),
                ),
            )
            .await?;
            tx.commit().await?;
            return Ok(Rejected);
        }
    }

    let payload = payload_for(&t, to, state.config.compliance_amount_threshold_minor)?;
    apply(&mut tx, &t, actor_id, from, to, payload).await?;
    tx.commit().await?;
    Ok(to)
}

async fn status_of(state: &AppState, id: Uuid) -> ApiResult<TransferStatus> {
    repo::get_owner_and_status(&state.pool, &id.to_string())
        .await?
        .map(|o| o.current_status)
        .ok_or_else(|| ApiError::not_found("Transfer not found."))
}

/// Applies one transition, treating a compliance hold as a stopping point
/// (`Ok(from)`, unchanged) rather than an error — for the internal drive
/// loops below, which advance as far as the transfer can go on its own.
async fn advance_once_or_hold(
    state: &AppState,
    transfer_id: Uuid,
    from: TransferStatus,
) -> ApiResult<TransferStatus> {
    match advance_once(state, transfer_id, None).await {
        Err(e) if e.code == ErrorCode::ComplianceHold => Ok(from),
        other => other,
    }
}

/// Drives a just-created transfer through the internal checks to AWAITING_FUNDS.
/// Stops early, still SCREENED, if compliance screening holds it for review.
pub async fn advance_to_awaiting_funds(state: &AppState, id: Uuid) -> ApiResult<TransferStatus> {
    let mut status = status_of(state, id).await?;
    while matches!(status, Created | Quoted | Screened) {
        let next = advance_once_or_hold(state, id, status).await?;
        if next == status {
            break;
        }
        status = next;
        if status.is_terminal() {
            break;
        }
    }
    Ok(status)
}

/// Drives a transfer forward until terminal or no forward step remains.
pub async fn advance_to_completion(
    state: &AppState,
    id: Uuid,
    step_delay_ms: u64,
) -> ApiResult<TransferStatus> {
    let mut status = status_of(state, id).await?;
    while forward_step(status).is_some() {
        if step_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(step_delay_ms)).await;
        }
        let next = advance_once_or_hold(state, id, status).await?;
        if next == status {
            break;
        }
        status = next;
        if status.is_terminal() {
            break;
        }
    }
    Ok(status)
}

/// Pre-funding timeout. No ledger effect.
pub async fn expire_transfer(state: &AppState, id: Uuid, actor_id: Option<Uuid>) -> ApiResult<()> {
    let mut tx = state.pool.begin().await?;
    let t = lock(&mut tx, id).await?;
    let from = TransferStatus::parse(&t.current_status)?;
    apply(&mut tx, &t, actor_id, from, Expired, None).await?;
    tx.commit().await?;
    Ok(())
}

/// Post-completion unwind: REVERSING → REVERSED with one compensating entry
/// (+sendAmount back to the send account). Only valid from COMPLETED.
pub async fn reverse_transfer(
    state: &AppState,
    id: Uuid,
    reason: &str,
    actor_id: Option<Uuid>,
) -> ApiResult<String> {
    let mut tx = state.pool.begin().await?;
    let t = lock(&mut tx, id).await?;
    let from = TransferStatus::parse(&t.current_status)?;

    apply(
        &mut tx,
        &t,
        actor_id,
        from,
        Reversing,
        Some(json!({ "reason": reason })),
    )
    .await?;

    let funding_entry_id: Option<Uuid> = sqlx::query_scalar(
        "select id from ledger_entries
          where transfer_id = $1 and amount_minor < 0 and description like '%funded%'
          order by posted_at limit 1",
    )
    .bind(t.id)
    .fetch_optional(&mut *tx)
    .await?;

    let send_ccy = CurrencyCode::parse(&t.send_currency)?;
    let send_acct = get_or_create_account(&mut tx, t.customer_id, send_ccy).await?;
    let (entry_id, _) = post_ledger_entry(
        &mut tx,
        LedgerPosting {
            account_id: send_acct,
            transfer_id: t.id,
            amount_minor: t.send_amount_minor,
            currency: send_ccy,
            description: &format!("Transfer {} — reversed", t.reference),
            reversal_of_entry_id: funding_entry_id,
        },
    )
    .await?;

    apply(
        &mut tx,
        &t,
        actor_id,
        Reversing,
        Reversed,
        Some(json!({ "reason": reason, "reversalLedgerEntryId": entry_id.to_string() })),
    )
    .await?;

    tx.commit().await?;
    Ok(entry_id.to_string())
}

/// A compliance analyst's disposition of a held SCREENED transfer.
pub enum ScreeningDecision {
    Clear,
    Reject,
}

/// Ops decision on a held transfer: clears it on to AWAITING_FUNDS, or
/// rejects it outright. Stub back-office action — reachable by any session
/// today, pending real operator-role auth (tracked separately); the hold
/// this resolves is real.
pub async fn resolve_screening_hold(
    state: &AppState,
    id: Uuid,
    decision: ScreeningDecision,
    reason: Option<String>,
    actor_id: Option<Uuid>,
) -> ApiResult<TransferStatus> {
    let mut tx = state.pool.begin().await?;
    let t = lock(&mut tx, id).await?;
    let from = TransferStatus::parse(&t.current_status)?;
    if from != Screened || !is_held(&t) {
        return Err(ApiError::conflict(
            "This transfer is not currently on a compliance hold.",
        ));
    }

    let (to, payload) = match decision {
        ScreeningDecision::Clear => (
            AwaitingFunds,
            json!({
                "fundingReference": tagged_reference("FR"),
                "holdResolution": "cleared",
                "holdResolutionReason": reason,
            }),
        ),
        ScreeningDecision::Reject => (
            Rejected,
            json!({
                "failureCategory": "compliance_hold",
                "reasonCode": "COMPLIANCE_HOLD",
                "reason": reason,
            }),
        ),
    };

    apply(&mut tx, &t, actor_id, from, to, Some(payload)).await?;
    tx.commit().await?;
    Ok(to)
}

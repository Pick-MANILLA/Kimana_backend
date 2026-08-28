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
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::tagged_reference;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct LockedTransfer {
    id: Uuid,
    reference: String,
    customer_id: Uuid,
    recipient_id: Uuid,
    send_currency: String,
    receive_currency: String,
    send_amount_minor: i64,
    receive_amount_minor: i64,
    current_status: String,
}

async fn lock(conn: &mut sqlx::PgConnection, id: Uuid) -> ApiResult<LockedTransfer> {
    sqlx::query_as::<_, LockedTransfer>(
        "select id, reference, customer_id, recipient_id, send_currency, receive_currency,
                send_amount_minor, receive_amount_minor, current_status
           from transfers where id = $1 for update",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?
    .ok_or_else(|| ApiError::not_found("Transfer not found."))
}

fn payload_for(to: TransferStatus) -> Option<Value> {
    match to {
        Screened => Some(json!({ "hold": false })),
        AwaitingFunds => Some(json!({ "fundingReference": tagged_reference("FR") })),
        Completed => Some(json!({ "payoutReference": tagged_reference("PO") })),
        _ => None,
    }
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

    apply(&mut tx, &t, actor_id, from, to, payload_for(to)).await?;
    tx.commit().await?;
    Ok(to)
}

async fn status_of(state: &AppState, id: Uuid) -> ApiResult<TransferStatus> {
    repo::get_owner_and_status(&state.pool, &id.to_string())
        .await?
        .map(|o| o.current_status)
        .ok_or_else(|| ApiError::not_found("Transfer not found."))
}

/// Drives a just-created transfer through the internal checks to AWAITING_FUNDS.
pub async fn advance_to_awaiting_funds(state: &AppState, id: Uuid) -> ApiResult<TransferStatus> {
    let mut status = status_of(state, id).await?;
    while matches!(status, Created | Quoted | Screened) {
        let next = advance_once(state, id, None).await?;
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
        let next = advance_once(state, id, None).await?;
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

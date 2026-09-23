#![allow(clippy::inconsistent_digit_grouping)]
//! How confirmed vault events move a transfer (`engine::apply_onchain`), and
//! that the simulator leaves SETTLING to the chain when settlement is on-chain.
//! Postgres only; the listener that feeds these events is covered against
//! Anvil in tests/settlement_listener.rs.

mod common;

use common::TestApp;
use kimana_backend::config::Config;
use kimana_backend::contract::transfer::TransferStatus;
use kimana_backend::domain::transfers::engine::{self, OnchainOutcome, OnchainTarget};
use kimana_backend::settlement::units::transfer_ref;
use serde_json::json;
use serial_test::file_serial;
use uuid::Uuid;

const RECIPIENT: &str = "00000000-0000-4000-8000-000000000020";

async fn onchain_app() -> TestApp {
    let mut config = Config::test();
    config.settlement_onchain = true;
    TestApp::with_config(config).await
}

async fn create_transfer(app: &TestApp, send_minor: i64) -> Uuid {
    let (_, quote) = app
        .post(
            "/quotes",
            json!({
                "sendCurrency": "USD", "receiveCurrency": "NGN",
                "amount": { "amountMinor": send_minor, "currency": "USD" }, "amountField": "send"
            }),
        )
        .await;
    let (_, t) = app
        .post(
            "/transfers",
            json!({
                "idempotencyKey": format!("onchain-{send_minor}-key"),
                "quoteId": quote["id"], "recipientId": RECIPIENT
            }),
        )
        .await;
    Uuid::parse_str(t["id"].as_str().unwrap()).unwrap()
}

/// AWAITING_FUNDS -> FUNDED -> SETTLING, where the on-chain gate stops it.
async fn to_settling(app: &TestApp, id: Uuid) {
    for _ in 0..3 {
        engine::advance_once(&app.state, id, None).await.unwrap();
    }
    assert_eq!(status(app, id).await, "SETTLING");
}

async fn status(app: &TestApp, id: Uuid) -> String {
    sqlx::query_scalar("select current_status from transfers where id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn apply(app: &TestApp, id: Uuid, target: OnchainTarget) -> OnchainOutcome {
    let mut tx = app.pool.begin().await.unwrap();
    let outcome = engine::apply_onchain(&mut tx, id, target, json!({ "event": "test" }))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    outcome
}

async fn ledger_sum(app: &TestApp, id: Uuid, currency: &str) -> i64 {
    sqlx::query_scalar(
        "select coalesce(sum(amount_minor), 0)::bigint from ledger_entries
          where transfer_id = $1 and currency = $2",
    )
    .bind(id)
    .bind(currency)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

#[tokio::test]
#[file_serial]
async fn transfer_stores_its_settlement_ref() {
    let app = onchain_app().await;
    let id = create_transfer(&app, 100_00).await;
    let stored: Vec<u8> = sqlx::query_scalar("select settlement_ref from transfers where id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stored, transfer_ref(&id.to_string()).to_vec());
}

#[tokio::test]
#[file_serial]
async fn settling_waits_for_the_chain() {
    let app = onchain_app().await;
    let id = create_transfer(&app, 100_00).await;
    to_settling(&app, id).await;

    let after = engine::advance_once(&app.state, id, None).await.unwrap();
    assert_eq!(
        after,
        TransferStatus::Settling,
        "the simulator must not settle"
    );

    assert_eq!(
        apply(&app, id, OnchainTarget::Settled).await,
        OnchainOutcome::Applied(TransferStatus::Settled)
    );
    assert_eq!(
        apply(&app, id, OnchainTarget::Settled).await,
        OnchainOutcome::AlreadyApplied(TransferStatus::Settled),
        "a replayed event changes nothing"
    );
}

#[tokio::test]
#[file_serial]
async fn failed_payout_unwinds_the_ledger() {
    let app = onchain_app().await;
    let id = create_transfer(&app, 200_00).await;
    to_settling(&app, id).await;
    apply(&app, id, OnchainTarget::Settled).await;
    assert!(ledger_sum(&app, id, "NGN").await > 0);

    assert_eq!(
        apply(&app, id, OnchainTarget::Reversing).await,
        OnchainOutcome::Applied(TransferStatus::Reversing)
    );
    assert_eq!(ledger_sum(&app, id, "NGN").await, 0, "conversion undone");

    assert_eq!(
        apply(&app, id, OnchainTarget::Reversed).await,
        OnchainOutcome::Applied(TransferStatus::Reversed)
    );
    assert_eq!(ledger_sum(&app, id, "USD").await, 0, "customer made whole");
    assert_eq!(
        apply(&app, id, OnchainTarget::Reversed).await,
        OnchainOutcome::AlreadyApplied(TransferStatus::Reversed)
    );
}

#[tokio::test]
#[file_serial]
async fn cancel_picks_expired_or_rejected() {
    let app = onchain_app().await;

    let expired = create_transfer(&app, 300_00).await;
    assert_eq!(
        apply(
            &app,
            expired,
            OnchainTarget::Cancelled {
                quote_expired: true
            }
        )
        .await,
        OnchainOutcome::Applied(TransferStatus::Expired)
    );

    let rejected = create_transfer(&app, 301_00).await;
    assert_eq!(
        apply(
            &app,
            rejected,
            OnchainTarget::Cancelled {
                quote_expired: false
            }
        )
        .await,
        OnchainOutcome::Applied(TransferStatus::Rejected)
    );

    // Once funded, EXPIRED is no longer a legal move: a cancel rejects instead.
    let funded = create_transfer(&app, 302_00).await;
    engine::advance_once(&app.state, funded, None)
        .await
        .unwrap();
    assert_eq!(
        apply(
            &app,
            funded,
            OnchainTarget::Cancelled {
                quote_expired: true
            }
        )
        .await,
        OnchainOutcome::Applied(TransferStatus::Rejected)
    );

    assert_eq!(
        apply(
            &app,
            expired,
            OnchainTarget::Cancelled {
                quote_expired: false
            }
        )
        .await,
        OnchainOutcome::AlreadyApplied(TransferStatus::Expired),
        "the backend already cancelled it"
    );
}

#[tokio::test]
#[file_serial]
async fn an_event_that_does_not_fit_is_ignored() {
    let app = onchain_app().await;
    let id = create_transfer(&app, 400_00).await; // AWAITING_FUNDS
    assert_eq!(
        apply(&app, id, OnchainTarget::Reversing).await,
        OnchainOutcome::Ignored(TransferStatus::AwaitingFunds)
    );
    assert_eq!(status(&app, id).await, "AWAITING_FUNDS");
}

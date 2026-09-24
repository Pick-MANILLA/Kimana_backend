#![allow(clippy::inconsistent_digit_grouping)]
//! The settlement listener against kimana_contract's `make e2e` Anvil setup,
//! with Postgres: real vault events, confirmation depth, replays and restarts.
//!
//! Ignored by default: `scripts/settlement-e2e.sh` starts Anvil, deploys
//! LocalE2E.s.sol, unpauses the vault and runs this file with `--ignored`.
//! Keys are Anvil's public default accounts, as in tests/settlement_anvil.rs.

mod common;

use alloy::primitives::{Address, B256, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use chrono::Utc;
use common::TestApp;
use kimana_backend::config::Config;
use kimana_backend::domain::transfers::engine;
use kimana_backend::settlement::bindings::SettlementVault;
use kimana_backend::settlement::units::{cents_to_usdc, quote_id, transfer_ref, RateE8};
use kimana_backend::settlement::{
    ListenerConfig, LockQuote, SettlementClient, SettlementConfig, SettlementListener,
};
use serde_json::json;
use serial_test::file_serial;
use std::time::Duration;
use uuid::Uuid;

alloy::sol! {
    #[sol(rpc)]
    interface IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
    }
}

// === Anvil accounts (LocalE2E.s.sol roles)
const OPERATOR_PK: &str = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";
const PARTNER_PK: &str = "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba";
const ONRAMP: &str = "0x976EA74026E726554dB657fA54763abd0C3a0aa9";
const NGN_RATE: &str = "1645.25";
const RECIPIENT: &str = "00000000-0000-4000-8000-000000000020";
const CONFIRMATIONS: u64 = 3;

// === Harness

struct Harness {
    app: TestApp,
    chain: SettlementConfig,
    operator: SettlementClient,
}

impl Harness {
    async fn new() -> Self {
        let mut config = Config::test();
        config.settlement_onchain = true;
        let chain = SettlementConfig::from_env()
            .expect("set SETTLEMENT_RPC_URL and SETTLEMENT_VAULT_ADDRESS (see scripts/settlement-e2e.sh)")
            .expect("valid settlement config");
        let signer: PrivateKeySigner = OPERATOR_PK.parse().unwrap();
        Harness {
            app: TestApp::with_config(config).await,
            operator: SettlementClient::new(&chain, signer),
            chain,
        }
    }

    /// A fresh listener, as after a process restart: all it shares with an
    /// earlier one is what is in the database.
    fn listener(&self) -> SettlementListener {
        SettlementListener::new(
            self.app.pool.clone(),
            self.chain.rpc_url.clone(),
            ListenerConfig {
                vault: self.chain.vault,
                confirmations: CONFIRMATIONS,
                start_block: 0,
                poll: Duration::from_millis(100),
                max_range: 1_000,
            },
        )
    }

    async fn mine(&self, blocks: u64) {
        ProviderBuilder::new()
            .connect_http(self.chain.rpc_url.clone())
            .raw_request::<_, ()>("anvil_mine".into(), (U256::from(blocks),))
            .await
            .unwrap();
    }

    async fn create_transfer(&self, send_minor: i64) -> Uuid {
        let (_, quote) = self
            .app
            .post(
                "/quotes",
                json!({
                    "sendCurrency": "USD", "receiveCurrency": "NGN",
                    "amount": { "amountMinor": send_minor, "currency": "USD" }, "amountField": "send"
                }),
            )
            .await;
        let (_, t) = self
            .app
            .post(
                "/transfers",
                json!({
                    "idempotencyKey": format!("listener-{send_minor}-{}", Utc::now().timestamp_nanos_opt().unwrap()),
                    "quoteId": quote["id"], "recipientId": RECIPIENT
                }),
            )
            .await;
        Uuid::parse_str(t["id"].as_str().expect("transfer created")).unwrap()
    }

    async fn to_settling(&self, id: Uuid) {
        for _ in 0..3 {
            engine::advance_once(&self.app.state, id, None)
                .await
                .unwrap();
        }
        assert_eq!(self.status(id).await, "SETTLING");
    }

    fn quote(&self, send_minor: i64, rate: &str) -> LockQuote {
        LockQuote {
            quote_id: quote_id(&Uuid::new_v4().to_string()),
            receive_currency: *b"NGN",
            receive_decimals: 2,
            expires_at: Utc::now().timestamp() as u64 + 90,
            rate: rate.parse::<RateE8>().unwrap(),
            usdc_amount: cents_to_usdc(send_minor as u64),
            fee_usdc: U256::ZERO,
        }
    }

    async fn lock(&self, id: Uuid, q: &LockQuote) -> B256 {
        let r#ref = transfer_ref(&id.to_string());
        self.operator
            .lock_quote(r#ref, q)
            .await
            .expect("lock_quote");
        r#ref
    }

    async fn lock_and_settle(&self, id: Uuid, send_minor: i64) -> B256 {
        let q = self.quote(send_minor, NGN_RATE);
        let r#ref = self.lock(id, &q).await;
        self.operator
            .settle(r#ref, partner().address(), q.usdc_amount)
            .await
            .expect("settle");
        r#ref
    }

    /// The off-ramp partner's NGN payout failed: it sends the USDC back.
    async fn partner_returns(&self, r#ref: B256, amount: U256) {
        let provider = ProviderBuilder::new()
            .wallet(partner())
            .connect_http(self.chain.rpc_url.clone());
        let usdc = self.operator.vault().asset().call().await.unwrap();
        IERC20::new(usdc, &provider)
            .approve(self.chain.vault, amount)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
        SettlementVault::new(self.chain.vault, &provider)
            .returnSettlement(r#ref)
            .send()
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap();
    }

    async fn status(&self, id: Uuid) -> String {
        sqlx::query_scalar("select current_status from transfers where id = $1")
            .bind(id)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    async fn events(&self, id: Uuid) -> Vec<String> {
        sqlx::query_scalar(
            "select event from settlement_events where transfer_id = $1 order by block_number, log_index",
        )
        .bind(id)
        .fetch_all(&self.app.pool)
        .await
        .unwrap()
    }

    async fn count(&self, sql: &str, id: Uuid) -> i64 {
        sqlx::query_scalar(sql)
            .bind(id)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    async fn ledger_sum(&self, id: Uuid, currency: &str) -> i64 {
        sqlx::query_scalar(
            "select coalesce(sum(amount_minor), 0)::bigint from ledger_entries
              where transfer_id = $1 and currency = $2",
        )
        .bind(id)
        .bind(currency)
        .fetch_one(&self.app.pool)
        .await
        .unwrap()
    }
}

fn partner() -> PrivateKeySigner {
    PARTNER_PK.parse().unwrap()
}

// === Tests

#[tokio::test]
#[file_serial]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn happy_path_waits_for_confirmations_then_settles() {
    let h = Harness::new().await;
    let id = h.create_transfer(150_00).await;
    h.to_settling(id).await;
    h.lock_and_settle(id, 150_00).await;

    let listener = h.listener();
    listener.run_once().await.unwrap();
    assert_eq!(
        h.status(id).await,
        "SETTLING",
        "not yet {CONFIRMATIONS} blocks deep"
    );
    assert!(h.events(id).await.is_empty());

    h.mine(CONFIRMATIONS).await;
    listener.run_once().await.unwrap();
    assert_eq!(h.status(id).await, "SETTLED");
    assert_eq!(h.events(id).await, ["QuoteLocked", "SettlementInitiated"]);
    assert!(h.ledger_sum(id, "NGN").await > 0);
}

#[tokio::test]
#[file_serial]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn refund_path_reverses_and_makes_the_customer_whole() {
    let h = Harness::new().await;
    let id = h.create_transfer(250_00).await;
    h.to_settling(id).await;
    let r#ref = h.lock_and_settle(id, 250_00).await;
    h.mine(CONFIRMATIONS).await;
    h.listener().run_once().await.unwrap();
    assert_eq!(h.status(id).await, "SETTLED");

    h.partner_returns(r#ref, cents_to_usdc(250_00)).await;
    h.mine(CONFIRMATIONS).await;
    h.listener().run_once().await.unwrap();
    assert_eq!(h.status(id).await, "REVERSING");
    assert_eq!(h.ledger_sum(id, "NGN").await, 0, "conversion undone");

    let onramp: Address = ONRAMP.parse().unwrap();
    h.operator.refund(r#ref, onramp).await.expect("refund");
    h.mine(CONFIRMATIONS).await;
    h.listener().run_once().await.unwrap();
    assert_eq!(h.status(id).await, "REVERSED");
    assert_eq!(h.ledger_sum(id, "USD").await, 0, "send amount returned");
    assert_eq!(
        h.events(id).await,
        [
            "QuoteLocked",
            "SettlementInitiated",
            "SettlementReturned",
            "SettlementRefunded"
        ]
    );
}

#[tokio::test]
#[file_serial]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn cancelled_quote_rejects_the_transfer() {
    let h = Harness::new().await;
    let id = h.create_transfer(120_00).await; // AWAITING_FUNDS
    let q = h.quote(120_00, NGN_RATE);
    let r#ref = h.lock(id, &q).await;
    h.operator.cancel_quote(r#ref).await.expect("cancel_quote");
    h.mine(CONFIRMATIONS).await;
    h.listener().run_once().await.unwrap();

    // Cancelled before its expiry, so the backend treats it as rejected.
    assert_eq!(h.status(id).await, "REJECTED");
    assert_eq!(h.events(id).await, ["QuoteLocked", "QuoteCancelled"]);
}

#[tokio::test]
#[file_serial]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn divergence_alert_is_stored_on_the_transfer() {
    let h = Harness::new().await;
    let id = h.create_transfer(110_00).await;
    // 2% above the oracle's reference: locks, but the vault raises RateDivergence.
    h.lock(id, &h.quote(110_00, "1678.155")).await;
    h.mine(CONFIRMATIONS).await;
    h.listener().run_once().await.unwrap();

    assert_eq!(h.events(id).await, ["RateDivergence", "QuoteLocked"]);
    let deviation: String = sqlx::query_scalar(
        "select payload->>'deviationBps' from settlement_events
          where transfer_id = $1 and event = 'RateDivergence'",
    )
    .bind(id)
    .fetch_one(&h.app.pool)
    .await
    .unwrap();
    assert_eq!(deviation, "200");
    let audits = h
        .count(
            "select count(*) from audit_log
              where entity_id = $1::text and action = 'transfer.settlement_alert'",
            id,
        )
        .await;
    assert_eq!(audits, 1);
    assert_eq!(
        h.status(id).await,
        "AWAITING_FUNDS",
        "an alert moves nothing"
    );
}

#[tokio::test]
#[file_serial]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn resumes_after_restart_and_replays_safely() {
    let h = Harness::new().await;
    let first = h.create_transfer(130_00).await;
    let second = h.create_transfer(140_00).await;
    h.to_settling(first).await;
    h.to_settling(second).await;

    h.lock_and_settle(first, 130_00).await;
    h.mine(CONFIRMATIONS).await;
    let before_restart = h.listener();
    before_restart.run_once().await.unwrap();
    assert_eq!(h.status(first).await, "SETTLED");
    drop(before_restart);

    // Settled while the listener was down.
    h.lock_and_settle(second, 140_00).await;
    h.mine(CONFIRMATIONS).await;
    h.listener().run_once().await.unwrap();
    assert_eq!(h.status(second).await, "SETTLED", "picked up after restart");

    let history = "select count(*) from transfer_state_history where transfer_id = $1";
    let ledger = "select count(*) from ledger_entries where transfer_id = $1";
    let events = "select count(*) from settlement_events where transfer_id = $1";
    let mut snapshot = Vec::new();
    for id in [first, second] {
        for sql in [history, ledger, events] {
            snapshot.push(h.count(sql, id).await);
        }
    }

    // Lose the cursor, as if a crash happened before it was saved: every log replays.
    sqlx::query("update settlement_cursor set last_block = 0")
        .execute(&h.app.pool)
        .await
        .unwrap();
    // The stored hash is for the old block; point it at block 0 so the replay is not a "reorg".
    let genesis = ProviderBuilder::new()
        .connect_http(h.chain.rpc_url.clone())
        .get_block_by_number(0.into())
        .await
        .unwrap()
        .unwrap()
        .header
        .hash;
    sqlx::query("update settlement_cursor set last_block_hash = $1")
        .bind(genesis.as_slice())
        .execute(&h.app.pool)
        .await
        .unwrap();
    h.listener().run_once().await.unwrap();

    let mut after = Vec::new();
    for id in [first, second] {
        for sql in [history, ledger, events] {
            after.push(h.count(sql, id).await);
        }
    }
    assert_eq!(snapshot, after, "a replay must not duplicate anything");
}

#[tokio::test]
#[file_serial]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn a_changed_block_hash_stops_the_listener() {
    let h = Harness::new().await;
    h.mine(CONFIRMATIONS + 1).await;
    let listener = h.listener();
    let last = listener
        .run_once()
        .await
        .unwrap()
        .expect("processed blocks");

    // Simulate a deep reorg: the stored hash no longer matches the chain.
    sqlx::query("update settlement_cursor set last_block_hash = $1")
        .bind(B256::repeat_byte(0xee).as_slice())
        .execute(&h.app.pool)
        .await
        .unwrap();
    let err = listener.run_once().await.expect_err("must stop");
    assert!(err.to_string().contains(&format!("block {last}")), "{err}");
}

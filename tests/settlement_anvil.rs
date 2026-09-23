//! Backend-driven settlement against kimana_contract's `make e2e` Anvil setup.
//!
//! Ignored by default: it needs Anvil with `script/LocalE2E.s.sol` deployed
//! and the vault unpaused. `scripts/settlement-e2e.sh` does all of that and
//! then runs this file with `--ignored`.
//!
//! The keys below are Anvil's public default accounts, the same ones
//! LocalE2E.s.sol assigns to each role. They hold no real value.

use alloy::primitives::{Address, B256, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use chrono::Utc;
use kimana_backend::error::ApiError;
use kimana_backend::error::ErrorCode;
use kimana_backend::settlement::units::{cents_to_usdc, quote_id, transfer_ref, RateE8};
use kimana_backend::settlement::{LockQuote, SettlementClient, SettlementConfig, SettlementError};

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
/// `NGN_RATE` in LocalE2E.s.sol: the oracle's reference rate.
const NGN_RATE: &str = "1645.25";

// === Helpers

fn config() -> SettlementConfig {
    SettlementConfig::from_env()
        .expect(
            "set SETTLEMENT_RPC_URL and SETTLEMENT_VAULT_ADDRESS (see scripts/settlement-e2e.sh)",
        )
        .expect("valid settlement config")
}

fn operator() -> SettlementClient {
    let signer: PrivateKeySigner = OPERATOR_PK.parse().unwrap();
    SettlementClient::new(&config(), signer)
}

fn partner_signer() -> PrivateKeySigner {
    PARTNER_PK.parse().unwrap()
}

/// Unique per run, so the test can run repeatedly against one Anvil.
fn unique(label: &str) -> String {
    format!("{label}_{}", Utc::now().timestamp_nanos_opt().unwrap())
}

fn quote(uuid: &str, rate: &str, cents: u64) -> LockQuote {
    LockQuote {
        quote_id: quote_id(uuid),
        receive_currency: *b"NGN",
        receive_decimals: 2,
        expires_at: Utc::now().timestamp() as u64 + 90,
        rate: rate.parse::<RateE8>().unwrap(),
        usdc_amount: cents_to_usdc(cents),
        fee_usdc: cents_to_usdc(100),
    }
}

fn api_error(err: SettlementError) -> ApiError {
    ApiError::from(err)
}

// === Tests

#[tokio::test]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn lock_then_settle_then_refund() {
    let client = operator();
    let partner = partner_signer().address();
    let onramp: Address = ONRAMP.parse().unwrap();

    let transfer = unique("backend_txn");
    let r#ref = transfer_ref(&transfer);
    let q = quote(&unique("backend_quote"), NGN_RATE, 10_000); // $100.00

    client.lock_quote(r#ref, &q).await.expect("lock_quote");
    let locked = client.vault().getQuote(r#ref).call().await.unwrap();
    assert_eq!(locked.quoteId, q.quote_id);
    assert_eq!(locked.receiveAmountMinor, U256::from(16_452_500u64)); // NGN 164,525.00
    assert!(client.vault().isQuoteUsed(q.quote_id).call().await.unwrap());

    client
        .settle(r#ref, partner, q.usdc_amount)
        .await
        .expect("settle");
    let settled = client.vault().getSettlement(r#ref).call().await.unwrap();
    assert_eq!(settled.status, 1, "Settled");
    assert_eq!(settled.partner, partner);
    assert_eq!(settled.amount, q.usdc_amount);

    // The NGN payout fails: the partner returns the USDC, then the backend refunds.
    let cfg = config();
    let partner_provider = ProviderBuilder::new()
        .wallet(partner_signer())
        .connect_http(cfg.rpc_url.clone());
    let usdc_address = client.vault().asset().call().await.unwrap();
    IERC20::new(usdc_address, &partner_provider)
        .approve(cfg.vault, q.usdc_amount)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    kimana_backend::settlement::bindings::SettlementVault::new(cfg.vault, &partner_provider)
        .returnSettlement(r#ref)
        .send()
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();

    client.refund(r#ref, onramp).await.expect("refund");
    let refunded = client.vault().getSettlement(r#ref).call().await.unwrap();
    assert_eq!(refunded.status, 3, "Refunded");
}

#[tokio::test]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn a_quote_id_locks_once() {
    let client = operator();
    let q = quote(&unique("backend_quote"), NGN_RATE, 5_000);
    client
        .lock_quote(transfer_ref(&unique("backend_txn")), &q)
        .await
        .expect("first lock");

    let err = client
        .lock_quote(transfer_ref(&unique("backend_txn")), &q)
        .await
        .expect_err("a reused quote id must be rejected");
    assert!(matches!(err, SettlementError::Vault(_)), "{err:?}");
    assert_eq!(api_error(err).code, ErrorCode::Conflict);
}

#[tokio::test]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn a_divergent_rate_is_blocked() {
    let client = operator();
    // 6% above the reference; the vault blocks anything over 5%.
    let q = quote(&unique("backend_quote"), "1743.965", 5_000);
    let err = client
        .lock_quote(transfer_ref(&unique("backend_txn")), &q)
        .await
        .expect_err("a divergent rate must be blocked");
    let api = api_error(err);
    assert_eq!(api.code, ErrorCode::PartnerFailure);
    assert!(!api.retryable);
    assert!(!client.vault().isQuoteUsed(q.quote_id).call().await.unwrap());
}

#[tokio::test]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn a_cancelled_transfer_cannot_settle() {
    let client = operator();
    let r#ref = transfer_ref(&unique("backend_txn"));
    let q = quote(&unique("backend_quote"), NGN_RATE, 5_000);
    client.lock_quote(r#ref, &q).await.expect("lock_quote");
    client.cancel_quote(r#ref).await.expect("cancel_quote");

    let partner = partner_signer().address();
    let err = client
        .settle(r#ref, partner, q.usdc_amount)
        .await
        .expect_err("a cancelled quote must not settle");
    assert_eq!(api_error(err).code, ErrorCode::Conflict);
}

#[tokio::test]
#[ignore = "needs Anvil + LocalE2E.s.sol; run scripts/settlement-e2e.sh"]
async fn an_expired_quote_is_not_sent() {
    let client = operator();
    let mut q = quote(&unique("backend_quote"), NGN_RATE, 5_000);
    q.expires_at = Utc::now().timestamp() as u64 - 1;
    let err = client
        .lock_quote(B256::repeat_byte(1), &q)
        .await
        .expect_err("expired");
    assert!(matches!(err, SettlementError::QuoteExpiredLocally));
    assert_eq!(api_error(err).code, ErrorCode::RateExpired);
}

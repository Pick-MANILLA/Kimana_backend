//! Live checks against the partners' sandboxes (issue #79). Ignored by
//! default; they need credentials in `.env`:
//!
//! ```sh
//! cargo test --test partners_sandbox -- --ignored --nocapture
//! ```
//!
//! Yellow Card needs YELLOWCARD_API_KEY / _SECRET (and YELLOWCARD_BASE_URL
//! for anything but the sandbox). Bridge needs BRIDGE_API_KEY, a sandbox
//! customer in BRIDGE_SANDBOX_CUSTOMER_ID and a destination address.

use kimana_backend::config::Config;
use kimana_backend::partners::bridge::Bridge;
use kimana_backend::partners::yellowcard::{NewReceive, YellowCard};
use uuid::Uuid;

fn config() -> Config {
    dotenvy::dotenv().ok();
    Config::from_env()
}

#[tokio::test]
#[ignore = "needs Yellow Card sandbox credentials"]
async fn yellowcard_sandbox_receive_round_trip() {
    let yc = YellowCard::from_config(&config())
        .expect("set YELLOWCARD_API_KEY and YELLOWCARD_API_SECRET");

    let channel = yc
        .ngn_bank_channel()
        .await
        .expect("NG bank deposit channel");
    println!("channel: {channel}");

    let sequence_id = Uuid::new_v4().to_string();
    let (receive, raw) = yc
        .submit_receive(NewReceive {
            sequence_id: &sequence_id,
            amount_minor: 5_000_000,
            reason: "Kimana sandbox check",
            business_name: "Kimana Sandbox Ltd",
            email: None,
        })
        .await
        .expect("submit receive");
    println!("submitted: {}", serde_json::to_string_pretty(&raw).unwrap());
    assert_eq!(receive.sequence_id.as_deref(), Some(sequence_id.as_str()));
    assert!(
        receive
            .bank_info
            .as_ref()
            .and_then(|b| b.account_number.as_ref())
            .is_some(),
        "receive has no bank account for the payer"
    );

    let (fetched, raw) = yc.get_receive(&receive.id).await.expect("lookup receive");
    println!("fetched: {}", serde_json::to_string_pretty(&raw).unwrap());
    assert_eq!(fetched.id, receive.id);

    if fetched.status != "complete" {
        yc.cancel_receive(&receive.id)
            .await
            .expect("cancel receive");
    }
}

#[tokio::test]
#[ignore = "needs Bridge sandbox credentials and a sandbox customer"]
async fn bridge_sandbox_virtual_account() {
    let bridge = Bridge::from_config(&config()).expect("set BRIDGE_API_KEY");
    let customer = std::env::var("BRIDGE_SANDBOX_CUSTOMER_ID")
        .expect("set BRIDGE_SANDBOX_CUSTOMER_ID to a KYB-approved sandbox customer");

    let (account, raw) = bridge
        .create_virtual_account(&customer, &format!("kimana-sandbox-{}", Uuid::new_v4()))
        .await
        .expect("create virtual account");
    println!("{}", serde_json::to_string_pretty(&raw).unwrap());
    assert!(account
        .source_deposit_instructions
        .bank_account_number
        .is_some());
    assert!(account
        .source_deposit_instructions
        .bank_routing_number
        .is_some());
}

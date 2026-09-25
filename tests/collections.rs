#![allow(clippy::inconsistent_digit_grouping)]
mod common;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::TestApp;
use http_body_util::BodyExt;
use kimana_backend::config::Config;
use kimana_backend::domain::collections::link_bridge_account;
use kimana_backend::ids::DEMO_CUSTOMER_ID;
use kimana_backend::partners::bridge::{pkcs1v15_sha256, signed_hash};
use kimana_backend::partners::yellowcard;
use kimana_backend::util::hmac_sha256;
use rsa::pkcs8::{EncodePublicKey, LineEnding};
use rsa::RsaPrivateKey;
use serde_json::{json, Value};
use serial_test::file_serial;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const YC_KEY: &str = "yc-test-key";
const YC_SECRET: &str = "yc-test-secret";
const BRIDGE_KEY: &str = "bridge-test-key";
const VIRTUAL_ACCOUNT: &str = "va_test_123";

// ---- mock partners ----

#[derive(Default)]
struct Mock {
    receives: HashMap<String, Value>,
    cancelled: Vec<String>,
    refuse_cancel: bool,
    virtual_account_requests: Vec<(String, String, Value)>,
}

type Shared = Arc<Mutex<Mock>>;

/// Rejects a request whose `YcHmacV1` signature doesn't check out.
fn check_yc_auth(headers: &HeaderMap, method: &str, path: &str, body: Option<&[u8]>) -> bool {
    let timestamp = headers
        .get("x-yc-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let expected = format!(
        "YcHmacV1 {YC_KEY}:{}",
        yellowcard::sign(YC_SECRET, timestamp, path, method, body)
    );
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(expected.as_str())
}

async fn yc_channels(headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !check_yc_auth(&headers, "GET", "/business/channels", None) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    (
        StatusCode::OK,
        Json(json!({ "channels": [
            { "id": "ch-withdraw", "rampType": "withdraw", "channelType": "bank",
              "country": "NG", "currency": "NGN", "status": "active" },
            { "id": "ch-ke", "rampType": "deposit", "channelType": "bank",
              "country": "KE", "currency": "KES", "status": "active" },
            { "id": "ch-ng-bank", "rampType": "deposit", "channelType": "bank",
              "country": "NG", "currency": "NGN", "status": "active", "apiStatus": "active" },
        ]})),
    )
}

async fn yc_submit(
    State(mock): State<Shared>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    if !check_yc_auth(&headers, "POST", "/business/receive", Some(&body)) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    let request: Value = serde_json::from_slice(&body).unwrap();
    let mut mock = mock.lock().unwrap();
    let id = format!("yc-rcv-{}", mock.receives.len() + 1);
    let expires = chrono::Utc::now() + chrono::Duration::minutes(30);
    let receive = json!({
        "id": id,
        "sequenceId": request["sequenceId"],
        "channelId": request["channelId"],
        "status": "pending",
        "currency": "NGN",
        "amount": 30.4,
        "convertedAmount": request["localAmount"],
        "expiresAt": expires.to_rfc3339(),
        "bankInfo": { "name": "Test Bank NG", "accountNumber": "3012345678", "accountName": "Yellow Card Financial" },
        "source": { "accountName": "Rotterdam Cocoa BV" },
        "request": request,
    });
    mock.receives.insert(id, receive.clone());
    (StatusCode::CREATED, Json(receive))
}

async fn yc_get(
    State(mock): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if !check_yc_auth(&headers, "GET", &format!("/business/receive/{id}"), None) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    match mock.lock().unwrap().receives.get(&id) {
        Some(receive) => (StatusCode::OK, Json(receive.clone())),
        None => (StatusCode::NOT_FOUND, Json(json!({}))),
    }
}

async fn yc_cancel(
    State(mock): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let path = format!("/business/receive/{id}/cancel");
    if !check_yc_auth(&headers, "POST", &path, Some(&body)) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    let mut mock = mock.lock().unwrap();
    if mock.refuse_cancel {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "code": "CannotCancel" })),
        );
    }
    mock.cancelled.push(id.clone());
    if let Some(receive) = mock.receives.get_mut(&id) {
        receive["status"] = "cancelled".into();
    }
    (
        StatusCode::OK,
        Json(json!({ "id": id, "status": "cancelled" })),
    )
}

async fn bridge_create_va(
    State(mock): State<Shared>,
    Path(customer): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if headers.get("api-key").and_then(|v| v.to_str().ok()) != Some(BRIDGE_KEY) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    mock.lock()
        .unwrap()
        .virtual_account_requests
        .push((customer, key, body.clone()));
    (
        StatusCode::CREATED,
        Json(json!({
            "id": VIRTUAL_ACCOUNT,
            "status": "activated",
            "source_deposit_instructions": {
                "currency": "usd",
                "payment_rails": ["ach_push", "wire"],
                "bank_name": "Lead Bank",
                "bank_address": "1801 Main St., Kansas City, MO 64108",
                "bank_beneficiary_name": "Kimana Demo Exports",
                "bank_account_number": "900000000001",
                "bank_routing_number": "101019644",
            },
            "destination": body["destination"],
        })),
    )
}

struct Partners {
    mock: Shared,
    bridge_key: RsaPrivateKey,
}

async fn spawn_mock() -> (Shared, String) {
    let mock: Shared = Arc::default();
    let router = Router::new()
        .route("/business/channels", get(yc_channels))
        .route("/business/receive", post(yc_submit))
        .route("/business/receive/{id}", get(yc_get))
        .route("/business/receive/{id}/cancel", post(yc_cancel))
        .route("/customers/{id}/virtual_accounts", post(bridge_create_va))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (mock, base)
}

/// A test app wired to the mock Yellow Card and Bridge.
async fn app_with_partners() -> (TestApp, Partners) {
    let (mock, base) = spawn_mock().await;
    let bridge_key = RsaPrivateKey::new(&mut rand::thread_rng(), 1024).unwrap();
    let mut config = Config::test();
    config.yellowcard_api_key = Some(YC_KEY.into());
    config.yellowcard_api_secret = Some(YC_SECRET.into());
    config.yellowcard_base_url = base.clone();
    config.bridge_api_key = Some(BRIDGE_KEY.into());
    config.bridge_base_url = base;
    config.bridge_destination_address = Some("0x89c1EE7c7888154Fb82e868f36bA5Dd3d4D80Faf".into());
    config.bridge_webhook_public_key = Some(
        bridge_key
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap(),
    );
    let app = TestApp::with_config(config).await;
    (app, Partners { mock, bridge_key })
}

// ---- helpers ----

async fn create(
    app: &TestApp,
    key: &str,
    amount_minor: i64,
    currency: &str,
) -> (StatusCode, Value) {
    app.post(
        "/collections",
        json!({
            "idempotencyKey": key,
            "amount": { "amountMinor": amount_minor, "currency": currency },
            "payerName": "Rotterdam Cocoa BV",
            "note": "Invoice 2026-114",
        }),
    )
    .await
}

async fn post_raw(
    app: &TestApp,
    uri: &str,
    header: (&str, String),
    body: &[u8],
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header(header.0, header.1)
        .body(Body::from(body.to_vec()))
        .unwrap();
    let response = app.router_clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn yc_webhook(app: &TestApp, event: Value) -> (StatusCode, Value) {
    let body = event.to_string();
    let signature = B64.encode(hmac_sha256(YC_SECRET.as_bytes(), body.as_bytes()));
    post_raw(
        app,
        "/webhooks/yellowcard",
        ("x-yc-signature", signature),
        body.as_bytes(),
    )
    .await
}

fn complete_receive(partners: &Partners, receive_id: &str, converted_amount: f64) {
    let mut mock = partners.mock.lock().unwrap();
    let receive = mock.receives.get_mut(receive_id).unwrap();
    receive["status"] = "complete".into();
    receive["convertedAmount"] = converted_amount.into();
}

fn bridge_event(event_id: &str, kind: &str, deposit_id: &str, amount: &str, memo: &str) -> Value {
    json!({
        "api_version": "v0",
        "event_id": event_id,
        "event_category": "virtual_account.activity",
        "event_type": "virtual_account.activity.created",
        "event_object_id": format!("act-{event_id}"),
        "event_object": {
            "id": format!("act-{event_id}"),
            "type": kind,
            "amount": amount,
            "currency": "usdc",
            "deposit_id": deposit_id,
            "customer_id": "bridge-cust-1",
            "virtual_account_id": VIRTUAL_ACCOUNT,
            "created_at": "2026-09-25T10:00:00.000Z",
            "source": {
                "payment_rail": "wire",
                "sender_name": "Rotterdam Cocoa BV",
                "description": memo,
            },
        },
        "event_created_at": "2026-09-25T10:00:01.000Z",
    })
}

fn bridge_signature(partners: &Partners, timestamp_ms: i64, body: &[u8]) -> String {
    let timestamp = timestamp_ms.to_string();
    let signature = partners
        .bridge_key
        .sign(pkcs1v15_sha256(), &signed_hash(&timestamp, body))
        .unwrap();
    format!("t={timestamp},v0={}", B64.encode(signature))
}

async fn bridge_webhook(app: &TestApp, partners: &Partners, event: Value) -> (StatusCode, Value) {
    let body = event.to_string();
    let header = bridge_signature(
        partners,
        chrono::Utc::now().timestamp_millis(),
        body.as_bytes(),
    );
    post_raw(
        app,
        "/webhooks/bridge",
        ("x-webhook-signature", header),
        body.as_bytes(),
    )
    .await
}

async fn link(app: &TestApp) {
    link_bridge_account(&app.state, DEMO_CUSTOMER_ID, "bridge-cust-1")
        .await
        .expect("link Bridge account");
}

async fn balance(app: &TestApp, currency: &str) -> i64 {
    sqlx::query_scalar(
        "select coalesce(sum(le.amount_minor), 0)::bigint
           from accounts a join ledger_entries le on le.account_id = a.id
          where a.currency = $1",
    )
    .bind(currency)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn receive_id(app: &TestApp, collection_id: &str) -> String {
    sqlx::query_scalar("select provider_ref from collections where id = $1::uuid")
        .bind(collection_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

// ---- NGN via Yellow Card ----

#[tokio::test]
#[file_serial]
async fn ngn_request_opens_a_yellowcard_receive() {
    let (app, partners) = app_with_partners().await;
    let (status, body) = create(&app, "coll-key-0001", 5_000_000, "NGN").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "PENDING");
    assert_eq!(
        body["amount"],
        json!({ "amountMinor": 5_000_000, "currency": "NGN" })
    );
    let reference = body["reference"].as_str().unwrap();
    assert!(reference.starts_with("CL-"));
    assert_eq!(
        body["payIn"],
        json!({
            "method": "NG_BANK_TRANSFER",
            "bankName": "Test Bank NG",
            "accountName": "Yellow Card Financial",
            "accountNumber": "3012345678",
            "memo": reference,
        })
    );
    // The request closes with the Yellow Card account, not after 7 days.
    let expires =
        chrono::DateTime::parse_from_rfc3339(body["expiresAt"].as_str().unwrap()).unwrap();
    assert!(expires < chrono::Utc::now() + chrono::Duration::minutes(31));

    let id = body["id"].as_str().unwrap();
    let rid = receive_id(&app, id).await;
    let sent = partners.mock.lock().unwrap().receives[&rid]["request"].clone();
    assert_eq!(sent["sequenceId"], id);
    assert_eq!(sent["channelId"], "ch-ng-bank");
    assert_eq!(sent["localAmount"], 50_000.0);
    assert_eq!(sent["currency"], "NGN");
    assert_eq!(sent["forceAccept"], true);
    assert_eq!(sent["reason"], "Invoice 2026-114");

    let (status, fetched) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched, body);

    // Replaying the key returns the same request without a second receive.
    let (_, again) = create(&app, "coll-key-0001", 5_000_000, "NGN").await;
    assert_eq!(again["id"], body["id"]);
    assert_eq!(partners.mock.lock().unwrap().receives.len(), 1);
}

#[tokio::test]
#[file_serial]
async fn completed_receive_credits_ngn_once() {
    let (app, partners) = app_with_partners().await;
    let ngn_before = balance(&app, "NGN").await;
    let (_, created) = create(&app, "coll-key-0010", 5_000_000, "NGN").await;
    let id = created["id"].as_str().unwrap();
    let rid = receive_id(&app, id).await;

    // A pending receive is not money yet.
    let (status, ack) = yc_webhook(&app, json!({ "id": rid, "event": "RECEIVE.PENDING" })).await;
    assert_eq!(status, StatusCode::OK, "{ack}");
    assert_eq!(ack, json!({ "applied": false, "duplicate": false }));
    // Neither is a disbursement event.
    let (_, ack) = yc_webhook(&app, json!({ "id": "p-1", "event": "PAYMENT.COMPLETE" })).await;
    assert_eq!(ack["applied"], false);

    complete_receive(&partners, &rid, 50_000.0);
    let event =
        json!({ "id": rid, "sequenceId": id, "status": "complete", "event": "RECEIVE.COMPLETE" });
    let (status, ack) = yc_webhook(&app, event.clone()).await;
    assert_eq!(status, StatusCode::OK, "{ack}");
    assert_eq!(ack, json!({ "applied": true, "duplicate": false }));
    assert_eq!(balance(&app, "NGN").await, ngn_before + 5_000_000);

    let (_, paid) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(paid["status"], "PAID");
    assert_eq!(
        paid["payment"]["amount"],
        json!({ "amountMinor": 5_000_000, "currency": "NGN" })
    );
    assert_eq!(paid["payment"]["payerName"], "Rotterdam Cocoa BV");

    let (_, ack) = yc_webhook(&app, event).await;
    assert_eq!(ack, json!({ "applied": true, "duplicate": true }));
    assert_eq!(balance(&app, "NGN").await, ngn_before + 5_000_000);
    assert_eq!(
        app.scalar_i64("select count(*) from ledger_entries where inbound_payment_id is not null")
            .await,
        1
    );
    assert_eq!(
        app.scalar_i64(
            "select count(*) from audit_log where action = 'collection.payment_received'"
        )
        .await,
        1
    );
}

#[tokio::test]
#[file_serial]
async fn receive_is_credited_for_the_amount_received() {
    let (app, partners) = app_with_partners().await;
    let ngn_before = balance(&app, "NGN").await;
    let (_, created) = create(&app, "coll-key-0020", 5_000_000, "NGN").await;
    let id = created["id"].as_str().unwrap();
    let rid = receive_id(&app, id).await;

    complete_receive(&partners, &rid, 49_000.5);
    let (status, _) = yc_webhook(&app, json!({ "id": rid, "event": "RECEIVE.COMPLETE" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(balance(&app, "NGN").await, ngn_before + 4_900_050);
    let (_, paid) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(paid["status"], "PAID");
    assert_eq!(paid["amount"]["amountMinor"], 5_000_000);
    assert_eq!(paid["payment"]["amount"]["amountMinor"], 4_900_050);
}

#[tokio::test]
#[file_serial]
async fn yellowcard_webhook_checks_the_signature() {
    let (app, partners) = app_with_partners().await;
    let (_, created) = create(&app, "coll-key-0030", 1_000_000, "NGN").await;
    let rid = receive_id(&app, created["id"].as_str().unwrap()).await;
    complete_receive(&partners, &rid, 10_000.0);

    let body = json!({ "id": rid, "event": "RECEIVE.COMPLETE" }).to_string();
    let wrong = B64.encode(hmac_sha256(b"not-the-secret", body.as_bytes()));
    let (status, _) = post_raw(
        &app,
        "/webhooks/yellowcard",
        ("x-yc-signature", wrong),
        body.as_bytes(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = post_raw(
        &app,
        "/webhooks/yellowcard",
        ("x-other", String::new()),
        body.as_bytes(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        app.scalar_i64("select count(*) from inbound_payments")
            .await,
        0
    );

    // A completed receive we never opened is refused loudly.
    partners.mock.lock().unwrap().receives.insert(
        "yc-foreign".into(),
        json!({ "id": "yc-foreign", "status": "complete", "currency": "NGN", "convertedAmount": 10.0 }),
    );
    let (status, _) = yc_webhook(
        &app,
        json!({ "id": "yc-foreign", "event": "RECEIVE.COMPLETE" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[file_serial]
async fn cancelling_ngn_cancels_the_receive_first() {
    let (app, partners) = app_with_partners().await;
    let (_, created) = create(&app, "coll-key-0040", 1_000_000, "NGN").await;
    let id = created["id"].as_str().unwrap();
    let rid = receive_id(&app, id).await;

    partners.mock.lock().unwrap().refuse_cancel = true;
    let (status, body) = app
        .post(&format!("/collections/{id}/cancel"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    let (_, still) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(still["status"], "PENDING");

    partners.mock.lock().unwrap().refuse_cancel = false;
    let (status, body) = app
        .post(&format!("/collections/{id}/cancel"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "CANCELLED");
    assert_eq!(partners.mock.lock().unwrap().cancelled, vec![rid.clone()]);
    let (status, _) = app
        .post(&format!("/collections/{id}/cancel"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "cancelling twice is a no-op");

    // Money that still arrives is credited and settles the request.
    complete_receive(&partners, &rid, 10_000.0);
    let (_, ack) = yc_webhook(&app, json!({ "id": rid, "event": "RECEIVE.COMPLETE" })).await;
    assert_eq!(ack["applied"], true);
    let (_, paid) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(paid["status"], "PAID");
}

#[tokio::test]
#[file_serial]
async fn ngn_is_unavailable_without_yellowcard() {
    let app = TestApp::new().await;
    let (status, body) = create(&app, "coll-key-0050", 1_000_000, "NGN").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["message"].as_str().unwrap().contains("NGN"));
    let (status, _) = yc_webhook(&app, json!({ "id": "x", "event": "RECEIVE.COMPLETE" })).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- USD via Bridge ----

#[tokio::test]
#[file_serial]
async fn usd_needs_a_linked_bridge_account() {
    let (app, partners) = app_with_partners().await;
    let (status, body) = create(&app, "coll-key-0060", 25_000, "USD").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (_, accounts) = app.get("/receiving-accounts").await;
    assert_eq!(accounts, json!([]));

    link(&app).await;
    {
        let mock = partners.mock.lock().unwrap();
        let (customer, key, body) = &mock.virtual_account_requests[0];
        assert_eq!(customer, "bridge-cust-1");
        assert_eq!(key, &format!("kimana-va-{DEMO_CUSTOMER_ID}"));
        assert_eq!(body["source"]["currency"], "usd");
        assert_eq!(body["destination"]["currency"], "usdc");
        assert_eq!(body["destination"]["payment_rail"], "base");
    }
    let err = link_bridge_account(&app.state, DEMO_CUSTOMER_ID, "bridge-cust-2")
        .await
        .unwrap_err();
    assert_eq!(err.code, kimana_backend::error::ErrorCode::Conflict);

    let (_, accounts) = app.get("/receiving-accounts").await;
    assert_eq!(accounts[0]["currency"], "USD");
    assert_eq!(accounts[0]["payIn"]["routingNumber"], "101019644");
    assert!(accounts[0]["payIn"].get("memo").is_none());

    let (status, body) = create(&app, "coll-key-0061", 25_000, "USD").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["payIn"]["method"], "US_BANK_TRANSFER");
    assert_eq!(body["payIn"]["accountNumber"], "900000000001");
    assert_eq!(body["payIn"]["paymentRails"], json!(["ach_push", "wire"]));
    assert_eq!(body["payIn"]["memo"], body["reference"]);
}

#[tokio::test]
#[file_serial]
async fn deposit_quoting_a_reference_settles_the_request() {
    let (app, partners) = app_with_partners().await;
    link(&app).await;
    let usd_before = balance(&app, "USD").await;
    let (_, created) = create(&app, "coll-key-0070", 25_000, "USD").await;
    let id = created["id"].as_str().unwrap();
    // Banks often drop the hyphen.
    let memo = format!(
        "INV 114 {}",
        created["reference"].as_str().unwrap().replace('-', "")
    );

    // Funds seen at the bank aren't credited until delivered.
    let (_, ack) = bridge_webhook(
        &app,
        &partners,
        bridge_event("wh_1", "funds_received", "dep_1", "250.00", &memo),
    )
    .await;
    assert_eq!(ack["applied"], false);
    assert_eq!(balance(&app, "USD").await, usd_before);

    let (status, ack) = bridge_webhook(
        &app,
        &partners,
        bridge_event("wh_2", "payment_processed", "dep_1", "250.00", &memo),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ack}");
    assert_eq!(ack, json!({ "applied": true, "duplicate": false }));
    assert_eq!(balance(&app, "USD").await, usd_before + 25_000);
    let (_, paid) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(paid["status"], "PAID");
    assert_eq!(paid["payment"]["payerName"], "Rotterdam Cocoa BV");

    // Another event for the same deposit doesn't credit again.
    let (_, ack) = bridge_webhook(
        &app,
        &partners,
        bridge_event("wh_3", "payment_processed", "dep_1", "250.00", &memo),
    )
    .await;
    assert_eq!(ack["duplicate"], true);
    assert_eq!(balance(&app, "USD").await, usd_before + 25_000);

    // A second deposit quoting the paid request is credited, unlinked.
    let (_, ack) = bridge_webhook(
        &app,
        &partners,
        bridge_event("wh_4", "payment_processed", "dep_2", "10.5", &memo),
    )
    .await;
    assert_eq!(ack["applied"], true);
    assert_eq!(balance(&app, "USD").await, usd_before + 26_050);
    assert_eq!(
        app.scalar_i64("select count(*) from inbound_payments where collection_id is null")
            .await,
        1
    );
}

#[tokio::test]
#[file_serial]
async fn every_deposit_is_credited() {
    let (app, partners) = app_with_partners().await;
    link(&app).await;
    let usd_before = balance(&app, "USD").await;
    let (_, created) = create(&app, "coll-key-0080", 25_000, "USD").await;
    let id = created["id"].as_str().unwrap();
    sqlx::query(
        "update collections set expires_at = now() - interval '1 minute' where id = $1::uuid",
    )
    .bind(id)
    .execute(&app.pool)
    .await
    .unwrap();

    let (_, ack) = bridge_webhook(
        &app,
        &partners,
        bridge_event("wh_1", "payment_processed", "dep_9", "99.999", "no memo"),
    )
    .await;
    assert_eq!(ack["applied"], true);
    // Fractions of a cent are floored.
    assert_eq!(balance(&app, "USD").await, usd_before + 9_999);

    // A late deposit quoting the expired request is credited but doesn't reopen it.
    let memo = created["reference"].as_str().unwrap().to_string();
    let (_, ack) = bridge_webhook(
        &app,
        &partners,
        bridge_event("wh_2", "payment_processed", "dep_10", "250", &memo),
    )
    .await;
    assert_eq!(ack["applied"], true);
    assert_eq!(balance(&app, "USD").await, usd_before + 34_999);
    let (_, fetched) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(fetched["status"], "EXPIRED");
}

#[tokio::test]
#[file_serial]
async fn bridge_webhook_checks_signature_freshness_and_account() {
    let (app, partners) = app_with_partners().await;
    link(&app).await;
    let body = bridge_event("wh_1", "payment_processed", "dep_1", "5.00", "").to_string();
    let now = chrono::Utc::now().timestamp_millis();

    let tampered = body.replace("5.00", "500.00");
    let header = bridge_signature(&partners, now, body.as_bytes());
    let (status, _) = post_raw(
        &app,
        "/webhooks/bridge",
        ("x-webhook-signature", header),
        tampered.as_bytes(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let stale = bridge_signature(&partners, now - 11 * 60 * 1000, body.as_bytes());
    let (status, _) = post_raw(
        &app,
        "/webhooks/bridge",
        ("x-webhook-signature", stale),
        body.as_bytes(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        app.scalar_i64("select count(*) from inbound_payments")
            .await,
        0
    );

    let mut unlinked = bridge_event("wh_2", "payment_processed", "dep_2", "5.00", "");
    unlinked["event_object"]["virtual_account_id"] = "va_someone_else".into();
    let (status, _) = bridge_webhook(&app, &partners, unlinked).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- common ----

#[tokio::test]
#[file_serial]
async fn create_validates_input() {
    let (app, _partners) = app_with_partners().await;
    let (status, _) = create(&app, "short", 1_000, "NGN").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = create(&app, "coll-key-0090", 0, "NGN").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = create(&app, "coll-key-0091", 1_000, "EUR").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("EUR"));
    let (status, _) = app
        .post(
            "/collections",
            json!({
                "idempotencyKey": "coll-key-0092",
                "amount": { "amountMinor": 1_000, "currency": "NGN" },
                "expiresAt": "2020-01-01T00:00:00Z",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[file_serial]
async fn another_customers_request_is_not_found() {
    let (app, _partners) = app_with_partners().await;
    let (_, created) = create(&app, "coll-key-0100", 1_000_000, "NGN").await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = app
        .post(
            "/register",
            json!({
                "email": "other-exporter@example.com",
                "password": "correct-horse-battery",
                "displayName": "Other",
                "legalName": "Other Exports Ltd",
            }),
        )
        .await;
    assert!(status.is_success(), "register failed: {status}");

    let (status, _) = app.get(&format!("/collections/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .post(&format!("/collections/{id}/cancel"), Value::Null)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = app.get("/collections").await;
    assert_eq!(list, json!([]));
    let (status, _) = app.get("/collections/not-a-uuid").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[file_serial]
async fn collection_routes_require_a_session() {
    let app = TestApp::new().await;
    app.clear_cookie();
    for uri in ["/collections", "/receiving-accounts"] {
        let (status, _) = app.get(uri).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    let (status, _) = create(&app, "coll-key-0110", 1_000, "USD").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

//! Mirrors Kimana_frontend/src/api/mock/seed.ts `createMockStore()`.
//! Dev-only: truncates the app tables and rebuilds them.

use crate::contract::common::CurrencyCode;
use crate::contract::quote::{CostBreakdown, FirmQuote};
use crate::ids::*;
use serde_json::json;
use sqlx::types::Json;
use sqlx::PgPool;
use uuid::Uuid;

const OPENING_BALANCES: &[(Uuid, &str, i64)] = &[
    (DEMO_ACCOUNT_NGN, "NGN", 4_825_000_000),
    (DEMO_ACCOUNT_USD, "USD", 12_450_000),
    (DEMO_ACCOUNT_EUR, "EUR", 1_820_000),
];

const FX_RATES: &[(&str, f64, f64)] = &[
    ("USD/NGN", 1645.2, 0.32),
    ("EUR/NGN", 1802.5, -0.11),
    ("GBP/NGN", 2088.4, 0.18),
    ("GHS/NGN", 110.25, -0.44),
];

const RECIPIENTS: &[(Uuid, &str, &str, &str)] = &[
    (
        DEMO_RECIPIENT_AMSTERDAM,
        "Amsterdam Commodities BV",
        "NL",
        "USD",
    ),
    (DEMO_RECIPIENT_KERALA, "Kerala Spices Corp", "IN", "USD"),
    (
        DEMO_RECIPIENT_NATURALIA,
        "Naturalia Foods GmbH",
        "DE",
        "EUR",
    ),
    (
        DEMO_RECIPIENT_ROTTERDAM,
        "Rotterdam Grain Exchange",
        "NL",
        "USD",
    ),
    (
        DEMO_RECIPIENT_GUPTA,
        "Gupta Trading India Pvt Ltd",
        "IN",
        "USD",
    ),
];

struct SeedTransfer {
    reference: &'static str,
    recipient: Uuid,
    send_ccy: &'static str,
    recv_ccy: &'static str,
    rate: f64,
    send_minor: i64,
    recv_minor: i64,
    description: &'static str,
    status: &'static str,
    payload: serde_json::Value,
}

fn seed_transfers() -> Vec<SeedTransfer> {
    vec![
        SeedTransfer {
            reference: "TXN-8844",
            recipient: DEMO_RECIPIENT_AMSTERDAM,
            send_ccy: "USD",
            recv_ccy: "NGN",
            rate: 1645.0,
            send_minor: 4_500_000,
            recv_minor: 7_402_500_00,
            description: "Cashew export",
            status: "COMPLETED",
            payload: json!({ "payoutReference": "PO-8844" }),
        },
        SeedTransfer {
            reference: "TXN-8843",
            recipient: DEMO_RECIPIENT_KERALA,
            send_ccy: "USD",
            recv_ccy: "NGN",
            rate: 1645.0,
            send_minor: 1_850_000,
            recv_minor: 3_043_250_00,
            description: "Sesame export",
            status: "PAYING_OUT",
            payload: json!(null),
        },
        SeedTransfer {
            reference: "TXN-8842",
            recipient: DEMO_RECIPIENT_NATURALIA,
            send_ccy: "EUR",
            recv_ccy: "NGN",
            rate: 1802.5,
            send_minor: 2_200_000,
            recv_minor: 3_965_500_00,
            description: "Hibiscus export",
            status: "SCREENED",
            payload: json!({ "hold": false }),
        },
        SeedTransfer {
            reference: "TXN-8841",
            recipient: DEMO_RECIPIENT_ROTTERDAM,
            send_ccy: "USD",
            recv_ccy: "NGN",
            rate: 1645.0,
            send_minor: 7_800_000,
            recv_minor: 12_831_000_00,
            description: "Cocoa export",
            status: "COMPLETED",
            payload: json!({ "payoutReference": "PO-8841" }),
        },
        SeedTransfer {
            reference: "TXN-8840",
            recipient: DEMO_RECIPIENT_GUPTA,
            send_ccy: "USD",
            recv_ccy: "NGN",
            rate: 1645.0,
            send_minor: 1_200_000,
            recv_minor: 1_974_000_00,
            description: "Sesame export",
            status: "REVERSED",
            payload: json!({ "reason": "Partner returned funds — beneficiary account closed.", "reversalLedgerEntryId": "00000000-0000-4000-8000-0000000000ff" }),
        },
    ]
}

fn snapshot_quote(t: &SeedTransfer) -> FirmQuote {
    let send = CurrencyCode::parse(t.send_ccy).unwrap();
    let receive = CurrencyCode::parse(t.recv_ccy).unwrap();
    FirmQuote {
        id: format!("seed_quote_{}", t.reference),
        send_currency: send,
        receive_currency: receive,
        breakdown: CostBreakdown {
            rate: t.rate,
            fee: crate::contract::common::Money::new(0, send),
            send_amount: crate::contract::common::Money::new(t.send_minor, send),
            receive_amount: crate::contract::common::Money::new(t.recv_minor, receive),
        },
        issued_at: "2026-08-20T09:00:00.000Z".into(),
        expires_at: "2026-08-20T09:01:30.000Z".into(),
    }
}

pub async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "truncate audit_log, ledger_entries, transfer_state_history, transfers, quotes,
                  recipients, fx_rates, kyb_checks, onboarding_documents, onboarding_principals,
                  onboarding_applications, accounts, customers, users
         restart identity cascade",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "insert into users (id, role, display_name, operator_permissions)
         values ($1, 'customer', 'Chinonso', '{}')",
    )
    .bind(DEMO_USER_ID)
    .execute(&mut *tx)
    .await?;

    sqlx::query("insert into customers (id, legal_name, primary_user_id) values ($1, $2, $3)")
        .bind(DEMO_CUSTOMER_ID)
        .bind("Adunola Exports Ltd")
        .bind(DEMO_USER_ID)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "insert into onboarding_applications (id, customer_id, status) values ($1, $2, 'draft')",
    )
    .bind(DEMO_APPLICATION_ID)
    .bind(DEMO_CUSTOMER_ID)
    .execute(&mut *tx)
    .await?;

    for (id, currency, minor) in OPENING_BALANCES {
        sqlx::query("insert into accounts (id, customer_id, currency) values ($1, $2, $3)")
            .bind(id)
            .bind(DEMO_CUSTOMER_ID)
            .bind(currency)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "insert into ledger_entries
               (account_id, amount_minor, currency, running_balance_minor, description)
             values ($1, $2, $3, $2, 'Opening balance')",
        )
        .bind(id)
        .bind(minor)
        .bind(currency)
        .execute(&mut *tx)
        .await?;
    }

    for (pair, rate, change) in FX_RATES {
        sqlx::query("insert into fx_rates (pair, rate, change_percent_24h) values ($1, $2, $3)")
            .bind(pair)
            .bind(rate)
            .bind(change)
            .execute(&mut *tx)
            .await?;
    }

    for (id, name, country, currency) in RECIPIENTS {
        sqlx::query(
            "insert into recipients
               (id, customer_id, account_name, account_number, bank_code, bank_name, currency, country, validation_status)
             values ($1, $2, $3, '0000000000', '000', 'Partner Bank', $4, $5, 'valid')",
        )
        .bind(id)
        .bind(DEMO_CUSTOMER_ID)
        .bind(name)
        .bind(currency)
        .bind(country)
        .execute(&mut *tx)
        .await?;
    }

    for t in seed_transfers() {
        let quote = snapshot_quote(&t);
        let id: Uuid = sqlx::query_scalar(
            "insert into transfers
               (reference, customer_id, idempotency_key, recipient_id, send_currency, receive_currency,
                send_amount_minor, receive_amount_minor, trade_description, quote_snapshot, current_status)
             values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             returning id",
        )
        .bind(t.reference)
        .bind(DEMO_CUSTOMER_ID)
        .bind(format!("seed-{}", t.reference))
        .bind(t.recipient)
        .bind(t.send_ccy)
        .bind(t.recv_ccy)
        .bind(t.send_minor)
        .bind(t.recv_minor)
        .bind(t.description)
        .bind(Json(&quote))
        .bind(t.status)
        .fetch_one(&mut *tx)
        .await?;

        let payload = if t.payload.is_null() {
            None
        } else {
            Some(t.payload)
        };
        sqlx::query(
            "insert into transfer_state_history (transfer_id, position, status, payload)
             values ($1, 0, $2, $3)",
        )
        .bind(id)
        .bind(t.status)
        .bind(payload)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

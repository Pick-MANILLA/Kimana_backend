//! Server-composed dashboard aggregate.

use crate::contract::common::{CurrencyCode, Money};
use crate::contract::dashboard::{
    BalanceHighlight, BalanceHighlightTone, DashboardOverview, DashboardStats, PendingAction,
    PendingActionKind, WorkingCapitalOffer,
};
use crate::domain::{ledger, onboarding};
use crate::error::ApiResult;
use crate::http::Session;
use crate::state::AppState;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use sqlx::PgPool;
use uuid::Uuid;

const FALLBACK_BUSINESS_NAME: &str = "Adunola Exports Ltd";
const FALLBACK_ACCOUNT_ID: &str = "AEL-00029";

async fn count_in_progress(pool: &PgPool, customer_id: Uuid) -> ApiResult<i64> {
    let terminal: Vec<&str> = vec!["COMPLETED", "REJECTED", "EXPIRED", "REVERSED"];
    let n: i64 = sqlx::query_scalar(
        "select count(*)::bigint from transfers
          where customer_id = $1 and current_status <> all($2)",
    )
    .bind(customer_id)
    .bind(&terminal)
    .fetch_one(pool)
    .await?;
    Ok(n)
}

async fn usd_send_volume_30d(pool: &PgPool, customer_id: Uuid) -> ApiResult<i64> {
    let total: i64 = sqlx::query_scalar(
        "select coalesce(sum(send_amount_minor), 0)::bigint from transfers
          where customer_id = $1 and send_currency = 'USD'
            and created_at >= now() - interval '30 days'",
    )
    .bind(customer_id)
    .fetch_one(pool)
    .await?;
    Ok(total)
}

fn balance_highlights() -> Vec<BalanceHighlight> {
    vec![
        BalanceHighlight {
            currency: CurrencyCode::Ngn,
            secondary_line: Some("≈ USD 29,330".into()),
            delta_text: "+₦2.4M this month".into(),
            delta_tone: BalanceHighlightTone::Success,
        },
        BalanceHighlight {
            currency: CurrencyCode::Usd,
            secondary_line: Some("Pending: +$18,500".into()),
            delta_text: "TXN-8843 settling".into(),
            delta_tone: BalanceHighlightTone::Success,
        },
        BalanceHighlight {
            currency: CurrencyCode::Eur,
            secondary_line: Some("≈ USD 19,760".into()),
            delta_text: "TXN-8842 in progress".into(),
            delta_tone: BalanceHighlightTone::Warning,
        },
    ]
}

fn pending_actions() -> Vec<PendingAction> {
    vec![
        PendingAction {
            id: "pact_paar_8842".into(),
            title: "PAAR — TXN-8842".into(),
            subtitle: "Upload required".into(),
            kind: PendingActionKind::ActionRequired,
            transfer_id: Some("txn_8842".into()),
        },
        PendingAction {
            id: "pact_formq_8843".into(),
            title: "Form Q — Sesame export".into(),
            subtitle: "Pending review".into(),
            kind: PendingActionKind::InReview,
            transfer_id: Some("txn_8843".into()),
        },
        PendingAction {
            id: "pact_bol_8843".into(),
            title: "BoL — TXN-8843".into(),
            subtitle: "Submitted for verification".into(),
            kind: PendingActionKind::Submitted,
            transfer_id: Some("txn_8843".into()),
        },
    ]
}

pub async fn get_overview(state: &AppState, session: &Session) -> ApiResult<DashboardOverview> {
    let balances = ledger::get_balances(&state.pool, session.customer_id).await?;
    let app = onboarding::repo::find_by_customer(&state.pool, session.customer_id).await?;
    let in_progress = count_in_progress(&state.pool, session.customer_id).await?;
    let volume_30d = usd_send_volume_30d(&state.pool, session.customer_id).await?;

    let display_name = app
        .as_ref()
        .and_then(|a| a.principals.first())
        .and_then(|p| p.full_name.split_whitespace().next())
        .map(String::from)
        .unwrap_or_else(|| session.display_name.clone());

    let business_name = app
        .as_ref()
        .and_then(|a| a.business.as_ref())
        .map(|b| b.legal_name.clone())
        .unwrap_or_else(|| FALLBACK_BUSINESS_NAME.to_string());

    let account_id = app
        .as_ref()
        .and_then(|a| a.approved_summary.as_ref())
        .map(|s| s.account_id.clone())
        .unwrap_or_else(|| FALLBACK_ACCOUNT_ID.to_string());

    Ok(DashboardOverview {
        display_name,
        business_name,
        account_id,
        balances,
        balance_highlights: balance_highlights(),
        stats: DashboardStats {
            volume30d: Money::new(volume_30d, CurrencyCode::Usd),
            transfers_in_progress: in_progress,
            // Still placeholder — need settlement-timing data.
            payout_success_rate_percent: 98.3,
            avg_settlement_seconds: 402,
        },
        pending_actions: pending_actions(),
        working_capital_offer: Some(WorkingCapitalOffer {
            max_advance: Money::new(3_825_000, CurrencyCode::Usd),
            basis_description: "Against Amsterdam Commodities receivable".into(),
            monthly_rate_percent: 2.5,
        }),
    })
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/dashboard/overview", get(overview))
}

async fn overview(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<DashboardOverview>> {
    Ok(Json(get_overview(&state, &session).await?))
}

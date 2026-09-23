use super::service::{self, CreateTransferInput};
use crate::contract::transfer::{Transfer, TransferStatus, TransferTimeline};
use crate::error::{ApiResult, ErrorResponse};
use crate::http::{Body, Session};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/transfers", post(create).get(list))
        .route("/transfers/{id}", get(get_one))
        .route("/transfers/{id}/timeline", get(timeline))
        .route(
            "/transfers/{id}/screening/decision",
            post(screening_decision),
        )
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTransferBody {
    #[serde(default)]
    idempotency_key: Option<String>,
    quote_id: String,
    recipient_id: String,
}

#[utoipa::path(
    post,
    path = "/transfers",
    tag = "transfers",
    request_body = CreateTransferBody,
    params(
        ("Idempotency-Key" = Option<String>, Header, description = "Takes precedence over `idempotencyKey` in the body"),
    ),
    responses(
        (status = 201, description = "Transfer created, or the existing one for a replayed idempotency key", body = Transfer),
        (status = 400, description = "Validation error", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Quote or recipient not found", body = ErrorResponse),
        (status = 409, description = "Quote expired or already accepted (RATE_EXPIRED / CONFLICT)", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    session: Session,
    headers: HeaderMap,
    Body(body): Body<CreateTransferBody>,
) -> ApiResult<(StatusCode, Json<Transfer>)> {
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or(body.idempotency_key)
        .unwrap_or_default();

    let transfer = service::create_transfer(
        &state,
        &session,
        CreateTransferInput {
            idempotency_key,
            quote_id: body.quote_id,
            recipient_id: body.recipient_id,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(transfer)))
}

#[utoipa::path(
    get,
    path = "/transfers/{id}",
    tag = "transfers",
    params(("id" = String, Path, description = "Transfer id")),
    responses(
        (status = 200, description = "Transfer", body = Transfer),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Transfer not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn get_one(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<Transfer>> {
    Ok(Json(service::get_transfer(&state, &session, &id).await?))
}

#[utoipa::path(
    get,
    path = "/transfers/{id}/timeline",
    tag = "transfers",
    params(("id" = String, Path, description = "Transfer id")),
    responses(
        (status = 200, description = "State history", body = TransferTimeline),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Transfer not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn timeline(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<TransferTimeline>> {
    Ok(Json(service::get_timeline(&state, &session, &id).await?))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListQuery {
    /// Filter by status, e.g. `AWAITING_FUNDS`
    #[serde(default)]
    status: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreeningDecisionBody {
    /// `clear` or `reject`
    decision: String,
    #[serde(default)]
    reason: Option<String>,
}

#[utoipa::path(
    post,
    path = "/transfers/{id}/screening/decision",
    tag = "transfers",
    params(("id" = String, Path, description = "Transfer id")),
    request_body = ScreeningDecisionBody,
    responses(
        (status = 200, description = "Hold resolved", body = Transfer),
        (status = 400, description = "Validation error", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Transfer not found", body = ErrorResponse),
        (status = 409, description = "Transfer is not on a screening hold", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn screening_decision(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
    Body(body): Body<ScreeningDecisionBody>,
) -> ApiResult<Json<Transfer>> {
    Ok(Json(
        service::decide_screening(
            &state,
            &session,
            &id,
            service::ScreeningDecisionInput {
                decision: body.decision,
                reason: body.reason,
            },
        )
        .await?,
    ))
}

#[utoipa::path(
    get,
    path = "/transfers",
    tag = "transfers",
    params(ListQuery),
    responses(
        (status = 200, description = "Transfers for the signed-in customer", body = [Transfer]),
        (status = 400, description = "Unknown status filter", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    session: Session,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Transfer>>> {
    let status = match q.status {
        Some(s) => Some(TransferStatus::parse(&s)?),
        None => None,
    };
    Ok(Json(
        service::list_transfers(&state, &session, status).await?,
    ))
}

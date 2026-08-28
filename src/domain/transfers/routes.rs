use super::service::{self, CreateTransferInput};
use crate::contract::transfer::{Transfer, TransferStatus, TransferTimeline};
use crate::error::ApiResult;
use crate::http::{Body, Session};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/transfers", post(create).get(list))
        .route("/transfers/{id}", get(get_one))
        .route("/transfers/{id}/timeline", get(timeline))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTransferBody {
    #[serde(default)]
    idempotency_key: Option<String>,
    quote_id: String,
    recipient_id: String,
}

async fn create(
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

async fn get_one(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<Transfer>> {
    Ok(Json(service::get_transfer(&state, &session, &id).await?))
}

async fn timeline(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<TransferTimeline>> {
    Ok(Json(service::get_timeline(&state, &session, &id).await?))
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    status: Option<String>,
}

async fn list(
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

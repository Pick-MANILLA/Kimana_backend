use crate::contract::auth::SessionResponse;
use crate::http::Session;
use crate::state::AppState;
use axum::routing::get;
use axum::{Json, Router};

pub fn session_routes() -> Router<AppState> {
    Router::new().route("/session", get(get_session))
}

#[utoipa::path(
    get,
    path = "/session",
    tag = "auth",
    responses(
        (status = 200, description = "Current session", body = SessionResponse),
        (status = 401, description = "Not authenticated"),
    )
)]
pub(crate) async fn get_session(session: Session) -> Json<SessionResponse> {
    Json(SessionResponse {
        user_id: session.user_id.to_string(),
        role: session.role,
        display_name: session.display_name,
        operator_permissions: (!session.operator_permissions.is_empty())
            .then_some(session.operator_permissions),
    })
}

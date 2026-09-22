use super::service::{self, RegisterInput, SESSION_TTL_DAYS};
use crate::contract::auth::SessionResponse;
use crate::error::ApiResult;
use crate::http::{Body, SESSION_COOKIE_NAME};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use time::Duration;
use utoipa::ToSchema;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/logout", post(logout))
}

fn set_session_cookie(jar: CookieJar, state: &AppState, token: String) -> CookieJar {
    // `SameSite=None` is required for the frontend (Vercel) and this API
    // (Render) sitting on different domains in production — browsers drop
    // `Lax` cookies on cross-site fetch/XHR. `None` requires `Secure`, so it
    // only applies when `cookie_secure` is on; local HTTP dev stays `Lax`
    // (frontend/backend differ only by port there, which is same-site).
    let same_site = if state.config.cookie_secure {
        SameSite::None
    } else {
        SameSite::Lax
    };
    let cookie = Cookie::build((SESSION_COOKIE_NAME, token))
        .http_only(true)
        .path("/")
        .same_site(same_site)
        .secure(state.config.cookie_secure)
        .max_age(Duration::days(SESSION_TTL_DAYS))
        .build();
    jar.add(cookie)
}

fn to_response(session: crate::http::Session) -> SessionResponse {
    SessionResponse {
        user_id: session.user_id.to_string(),
        role: session.role,
        display_name: session.display_name,
        operator_permissions: (!session.operator_permissions.is_empty())
            .then_some(session.operator_permissions),
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterBody {
    email: String,
    password: String,
    display_name: String,
    legal_name: String,
}

#[utoipa::path(
    post,
    path = "/register",
    tag = "auth",
    request_body = RegisterBody,
    responses(
        (status = 201, description = "Account created", body = SessionResponse),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Email already registered"),
    )
)]
pub(crate) async fn register(
    State(state): State<AppState>,
    jar: CookieJar,
    Body(body): Body<RegisterBody>,
) -> ApiResult<(StatusCode, CookieJar, Json<SessionResponse>)> {
    let authenticated = service::register(
        &state,
        RegisterInput {
            email: body.email,
            password: body.password,
            display_name: body.display_name,
            legal_name: body.legal_name,
        },
    )
    .await?;

    let jar = set_session_cookie(jar, &state, authenticated.raw_token);
    Ok((
        StatusCode::CREATED,
        jar,
        Json(to_response(authenticated.session)),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginBody {
    email: String,
    password: String,
}

#[utoipa::path(
    post,
    path = "/login",
    tag = "auth",
    request_body = LoginBody,
    responses(
        (status = 200, description = "Authenticated", body = SessionResponse),
        (status = 401, description = "Invalid credentials"),
    )
)]
pub(crate) async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    Body(body): Body<LoginBody>,
) -> ApiResult<(CookieJar, Json<SessionResponse>)> {
    let authenticated = service::login(&state, &body.email, &body.password).await?;
    let jar = set_session_cookie(jar, &state, authenticated.raw_token);
    Ok((jar, Json(to_response(authenticated.session))))
}

#[utoipa::path(
    post,
    path = "/logout",
    tag = "auth",
    responses(
        (status = 204, description = "Session cleared"),
    )
)]
pub(crate) async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> ApiResult<(CookieJar, StatusCode)> {
    if let Some(token) = jar.get(SESSION_COOKIE_NAME).map(|c| c.value().to_string()) {
        service::logout(&state, &token).await?;
    }
    let jar = jar.remove(Cookie::from(SESSION_COOKIE_NAME));
    Ok((jar, StatusCode::NO_CONTENT))
}

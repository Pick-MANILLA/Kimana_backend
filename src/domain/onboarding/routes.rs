use super::service::{self, DocumentUpload};
use crate::contract::onboarding::{
    BusinessDetails, DirectorOrBeneficialOwner, OnboardingApplication, OnboardingDocumentType,
    UploadedDocument,
};
use crate::error::{ApiError, ApiResult, ErrorResponse};
use crate::http::{Body, Session};
use crate::state::AppState;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;

/// Document uploads carry file bytes, so they need more headroom than the
/// 128 KB default the rest of the JSON API is capped at (see ISSUE-BE-05) —
/// applied directly to this route so it overrides that default.
const DOCUMENT_UPLOAD_BODY_LIMIT: usize = 12 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/onboarding/application", get(get_application))
        .route("/onboarding/application/business", put(save_business))
        .route("/onboarding/application/principals", put(save_principals))
        .route("/onboarding/application/submit", post(submit))
        .route(
            "/onboarding/application/documents",
            post(upload_document).layer(DefaultBodyLimit::max(DOCUMENT_UPLOAD_BODY_LIMIT)),
        )
        .route(
            "/onboarding/application/documents/{id}/retry",
            post(retry_document),
        )
        .route(
            "/onboarding/application/documents/{id}",
            delete(remove_document),
        )
}

#[utoipa::path(
    get,
    path = "/onboarding/application",
    tag = "onboarding",
    responses(
        (status = 200, description = "The customer's onboarding application", body = OnboardingApplication),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "No application for this customer", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn get_application(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(service::get_application(&state, &session).await?))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveBusinessBody {
    #[serde(default)]
    application_id: Option<String>,
    business: BusinessDetails,
}

#[utoipa::path(
    put,
    path = "/onboarding/application/business",
    tag = "onboarding",
    request_body = SaveBusinessBody,
    responses(
        (status = 200, description = "Updated application", body = OnboardingApplication),
        (status = 400, description = "Validation error", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Application not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn save_business(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<SaveBusinessBody>,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(
        service::save_business_details(&state, &session, body.business, body.application_id)
            .await?,
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavePrincipalsBody {
    #[serde(default)]
    application_id: Option<String>,
    principals: Vec<DirectorOrBeneficialOwner>,
}

#[utoipa::path(
    put,
    path = "/onboarding/application/principals",
    tag = "onboarding",
    request_body = SavePrincipalsBody,
    responses(
        (status = 200, description = "Updated application", body = OnboardingApplication),
        (status = 400, description = "Validation error", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Application not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn save_principals(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<SavePrincipalsBody>,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(
        service::save_principals(&state, &session, body.principals, body.application_id).await?,
    ))
}

#[derive(Deserialize, Default, ToSchema)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct SubmitBody {
    application_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/onboarding/application/submit",
    tag = "onboarding",
    request_body = SubmitBody,
    responses(
        (status = 200, description = "Application after KYB: approved or rejected", body = OnboardingApplication),
        (status = 400, description = "Business details missing", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Application not found", body = ErrorResponse),
        (status = 409, description = "Already under review", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn submit(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<SubmitBody>,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(
        service::submit(&state, &session, body.application_id).await?,
    ))
}

/// OpenAPI-only shape of the multipart form `upload_document` reads by hand.
#[allow(dead_code)]
#[derive(ToSchema)]
#[schema(rename_all = "camelCase")]
pub(crate) struct DocumentUploadForm {
    /// `cac_certificate`, `memart`, `proof_of_address`, `directors_id` or `board_resolution`
    r#type: String,
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
    application_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/onboarding/application/documents",
    tag = "onboarding",
    request_body(content = DocumentUploadForm, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "Stored document", body = UploadedDocument),
        (status = 400, description = "Missing file or invalid type", body = ErrorResponse),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Application not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn upload_document(
    State(state): State<AppState>,
    session: Session,
    mut multipart: Multipart,
) -> ApiResult<Json<UploadedDocument>> {
    let mut bytes: Option<Vec<u8>> = None;
    let mut file_name = String::new();
    let mut mime_type = String::new();
    let mut doc_type: Option<String> = None;
    let mut application_id: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::validation(e.to_string()))?
    {
        let name = field.name().map(str::to_string);
        match name.as_deref() {
            Some("file") => {
                file_name = field.file_name().unwrap_or("upload").to_string();
                mime_type = field.content_type().unwrap_or("").to_string();
                bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::validation(e.to_string()))?
                        .to_vec(),
                );
            }
            Some("type") => {
                doc_type = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::validation(e.to_string()))?,
                );
            }
            Some("applicationId") => {
                application_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::validation(e.to_string()))?,
                );
            }
            _ => {}
        }
    }

    let doc_type = doc_type
        .as_deref()
        .and_then(OnboardingDocumentType::parse)
        .ok_or_else(|| ApiError::validation("Provide a valid document `type`."))?;
    let bytes = bytes.ok_or_else(|| ApiError::validation("No file was included in the upload."))?;

    let saved = service::upload_document(
        &state,
        &session,
        DocumentUpload {
            doc_type,
            file_name,
            mime_type,
            bytes,
        },
        application_id,
    )
    .await?;
    Ok(Json(saved))
}

#[utoipa::path(
    post,
    path = "/onboarding/application/documents/{id}/retry",
    tag = "onboarding",
    params(("id" = String, Path, description = "Document id")),
    responses(
        (status = 200, description = "Document after retry", body = UploadedDocument),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Document not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn retry_document(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<UploadedDocument>> {
    Ok(Json(
        service::retry_document_upload(&state, &session, &id).await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/onboarding/application/documents/{id}",
    tag = "onboarding",
    params(("id" = String, Path, description = "Document id")),
    responses(
        (status = 204, description = "Document removed"),
        (status = 401, description = "Not authenticated", body = ErrorResponse),
        (status = 404, description = "Document not found", body = ErrorResponse),
    ),
    security(("cookieAuth" = []))
)]
pub(crate) async fn remove_document(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    service::remove_document(&state, &session, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

use super::service::{self, DocumentUpload};
use crate::contract::onboarding::{
    BusinessDetails, DirectorOrBeneficialOwner, OnboardingApplication, OnboardingDocumentType,
    UploadedDocument,
};
use crate::error::{ApiError, ApiResult};
use crate::http::{Body, Session};
use crate::state::AppState;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/onboarding/application", get(get_application))
        .route("/onboarding/application/business", put(save_business))
        .route("/onboarding/application/principals", put(save_principals))
        .route("/onboarding/application/submit", post(submit))
        .route("/onboarding/application/documents", post(upload_document))
        .route(
            "/onboarding/application/documents/{id}/retry",
            post(retry_document),
        )
        .route(
            "/onboarding/application/documents/{id}",
            delete(remove_document),
        )
}

async fn get_application(
    State(state): State<AppState>,
    session: Session,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(service::get_application(&state, &session).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveBusinessBody {
    #[serde(default)]
    application_id: Option<String>,
    business: BusinessDetails,
}

async fn save_business(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<SaveBusinessBody>,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(
        service::save_business_details(&state, &session, body.business, body.application_id)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavePrincipalsBody {
    #[serde(default)]
    application_id: Option<String>,
    principals: Vec<DirectorOrBeneficialOwner>,
}

async fn save_principals(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<SavePrincipalsBody>,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(
        service::save_principals(&state, &session, body.principals, body.application_id).await?,
    ))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct SubmitBody {
    application_id: Option<String>,
}

async fn submit(
    State(state): State<AppState>,
    session: Session,
    Body(body): Body<SubmitBody>,
) -> ApiResult<Json<OnboardingApplication>> {
    Ok(Json(
        service::submit(&state, &session, body.application_id).await?,
    ))
}

async fn upload_document(
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

async fn retry_document(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<Json<UploadedDocument>> {
    Ok(Json(
        service::retry_document_upload(&state, &session, &id).await?,
    ))
}

async fn remove_document(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    service::remove_document(&state, &session, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

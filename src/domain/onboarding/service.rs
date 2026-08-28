use super::kyb;
use super::repo::{self, DocInput, StatusPatch};
use super::schema;
use crate::audit::{write_audit, AuditEntry};
use crate::contract::common::{CurrencyCode, Money};
use crate::contract::onboarding::{
    ApprovedAccountSummary, BusinessDetails, DirectorOrBeneficialOwner, OnboardingApplication,
    OnboardingDocumentType, UploadedDocument,
};
use crate::error::{ApiError, ApiResult};
use crate::http::Session;
use crate::state::AppState;
use crate::storage;
use crate::util::generate_account_id;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

const ALLOWED_MIME: [&str; 3] = ["application/pdf", "image/jpeg", "image/png"];
const MAX_FILE_BYTES: i64 = 10 * 1024 * 1024;

async fn load_owned(
    state: &AppState,
    session: &Session,
    application_id: Option<&str>,
) -> ApiResult<OnboardingApplication> {
    let app = repo::find_by_customer(&state.pool, session.customer_id)
        .await?
        .ok_or_else(|| ApiError::not_found("No onboarding application for this customer."))?;
    if let Some(id) = application_id {
        if id != app.id {
            return Err(ApiError::not_found("Application not found."));
        }
    }
    Ok(app)
}

pub async fn get_application(
    state: &AppState,
    session: &Session,
) -> ApiResult<OnboardingApplication> {
    load_owned(state, session, None).await
}

pub async fn save_business_details(
    state: &AppState,
    session: &Session,
    business: BusinessDetails,
    application_id: Option<String>,
) -> ApiResult<OnboardingApplication> {
    schema::validate_business(&business)?;
    let app = load_owned(state, session, application_id.as_deref()).await?;
    let app_uuid = Uuid::parse_str(&app.id).unwrap();

    let mut tx = state.pool.begin().await?;
    repo::save_business(&mut tx, app_uuid, &business).await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "onboarding.business_saved",
            entity_type: "onboarding_application",
            entity_id: app.id.clone(),
            before: app
                .business
                .as_ref()
                .map(|b| serde_json::to_value(b).unwrap()),
            after: Some(serde_json::to_value(&business)?),
        },
    )
    .await?;
    tx.commit().await?;

    load_owned(state, session, None).await
}

pub async fn save_principals(
    state: &AppState,
    session: &Session,
    principals: Vec<DirectorOrBeneficialOwner>,
    application_id: Option<String>,
) -> ApiResult<OnboardingApplication> {
    schema::validate_principals(&principals)?;
    let app = load_owned(state, session, application_id.as_deref()).await?;
    let app_uuid = Uuid::parse_str(&app.id).unwrap();

    let mut tx = state.pool.begin().await?;
    repo::replace_principals(&mut tx, app_uuid, &principals).await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "onboarding.principals_saved",
            entity_type: "onboarding_application",
            entity_id: app.id.clone(),
            before: Some(serde_json::to_value(&app.principals)?),
            after: Some(serde_json::to_value(&principals)?),
        },
    )
    .await?;
    tx.commit().await?;

    load_owned(state, session, None).await
}

pub struct DocumentUpload {
    pub doc_type: OnboardingDocumentType,
    pub file_name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

pub async fn upload_document(
    state: &AppState,
    session: &Session,
    upload: DocumentUpload,
    application_id: Option<String>,
) -> ApiResult<UploadedDocument> {
    let app = load_owned(state, session, application_id.as_deref()).await?;
    if !ALLOWED_MIME.contains(&upload.mime_type.as_str()) {
        return Err(ApiError::validation(
            "That file type isn't supported. Upload a PDF, JPG, or PNG.",
        ));
    }
    if upload.bytes.len() as i64 > MAX_FILE_BYTES {
        return Err(ApiError::validation(
            "That file is larger than 10 MB. Compress it or choose a smaller copy.",
        ));
    }
    let app_uuid = Uuid::parse_str(&app.id).unwrap();
    let previous = app
        .documents
        .iter()
        .find(|d| d.r#type == upload.doc_type.as_str())
        .cloned();

    let storage_key = format!(
        "onboarding/{}/{}/{}-{}",
        app.id,
        upload.doc_type.as_str(),
        Utc::now().timestamp_millis(),
        upload.file_name
    );
    storage::put(&state.config.storage_dir, &storage_key, &upload.bytes).await?;

    let mut tx = state.pool.begin().await?;
    let saved = repo::upsert_document(
        &mut tx,
        app_uuid,
        DocInput {
            doc_type: upload.doc_type.as_str(),
            file_name: &upload.file_name,
            mime_type: &upload.mime_type,
            size_bytes: upload.bytes.len() as i64,
            storage_key: &storage_key,
        },
    )
    .await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: if previous.is_some() {
                "onboarding.document_replaced"
            } else {
                "onboarding.document_uploaded"
            },
            entity_type: "onboarding_document",
            entity_id: saved.id.clone(),
            before: previous.as_ref().map(|d| serde_json::to_value(d).unwrap()),
            after: Some(serde_json::to_value(&saved)?),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(saved)
}

pub async fn retry_document_upload(
    state: &AppState,
    session: &Session,
    document_id: &str,
) -> ApiResult<UploadedDocument> {
    let app = load_owned(state, session, None).await?;
    let doc_uuid = parse_uuid_or_not_found(document_id)?;
    let found = repo::find_document(&state.pool, doc_uuid)
        .await?
        .filter(|f| f.application_id == Uuid::parse_str(&app.id).unwrap())
        .ok_or_else(|| ApiError::not_found("Document not found."))?;

    let mut tx = state.pool.begin().await?;
    let updated = repo::mark_document_uploaded(&mut tx, doc_uuid).await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "onboarding.document_retried",
            entity_type: "onboarding_document",
            entity_id: document_id.to_string(),
            before: Some(serde_json::to_value(&found.document)?),
            after: Some(serde_json::to_value(&updated)?),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(updated)
}

pub async fn remove_document(
    state: &AppState,
    session: &Session,
    document_id: &str,
) -> ApiResult<()> {
    let app = load_owned(state, session, None).await?;
    let doc_uuid = parse_uuid_or_not_found(document_id)?;
    let found = repo::find_document(&state.pool, doc_uuid)
        .await?
        .filter(|f| f.application_id == Uuid::parse_str(&app.id).unwrap())
        .ok_or_else(|| ApiError::not_found("Document not found."))?;

    let mut tx = state.pool.begin().await?;
    repo::delete_document(&mut tx, doc_uuid).await?;
    write_audit(
        &mut tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "onboarding.document_removed",
            entity_type: "onboarding_document",
            entity_id: document_id.to_string(),
            before: Some(serde_json::to_value(&found.document)?),
            after: None,
        },
    )
    .await?;
    tx.commit().await?;

    if let Some(key) = found.storage_key {
        let _ = storage::delete(&state.config.storage_dir, &key).await;
    }
    Ok(())
}

/// Slice: moves through submitted -> in_review (committed), runs the KYB
/// provider, then commits approved (+ approvedSummary) or rejected
/// (+ rejectionReasons). Resolves only at a terminal status — the frontend
/// does not poll.
pub async fn submit(
    state: &AppState,
    session: &Session,
    application_id: Option<String>,
) -> ApiResult<OnboardingApplication> {
    let app = load_owned(state, session, application_id.as_deref()).await?;
    let Some(business) = app.business.clone() else {
        return Err(ApiError::validation(
            "Add your business details before submitting.",
        ));
    };
    if app.status == "submitted" || app.status == "in_review" {
        return Err(ApiError::conflict(
            "This application is already being reviewed.",
        ));
    }
    let app_uuid = Uuid::parse_str(&app.id).unwrap();

    {
        let mut tx = state.pool.begin().await?;
        repo::patch_status(
            &mut tx,
            app_uuid,
            StatusPatch {
                status: "submitted".into(),
                mark_submitted: true,
                ..Default::default()
            },
        )
        .await?;
        audit_transition(&mut tx, session, &app.id, &app.status, "submitted").await?;
        repo::patch_status(
            &mut tx,
            app_uuid,
            StatusPatch {
                status: "in_review".into(),
                ..Default::default()
            },
        )
        .await?;
        audit_transition(&mut tx, session, &app.id, "submitted", "in_review").await?;
        tx.commit().await?;
    }

    let outcome = kyb::run_checks(&app, state.config.kyb_check_delay_ms).await;

    {
        let mut tx = state.pool.begin().await?;
        repo::replace_kyb_checks(&mut tx, app_uuid, &outcome.checks).await?;

        if outcome.approved {
            let summary = ApprovedAccountSummary {
                account_id: generate_account_id(&business.legal_name),
                risk_rating_label: "Medium-Low".into(),
                segment: business.industry.segment_label().into(),
                corridor: "NGN → USD / EUR".into(),
                monthly_limit: Money::new(100_000_00, CurrencyCode::Usd),
            };
            repo::patch_status(
                &mut tx,
                app_uuid,
                StatusPatch {
                    status: "approved".into(),
                    mark_reviewed: true,
                    approved_summary: Some(summary),
                    ..Default::default()
                },
            )
            .await?;
            audit_transition(&mut tx, session, &app.id, "in_review", "approved").await?;
        } else {
            repo::patch_status(
                &mut tx,
                app_uuid,
                StatusPatch {
                    status: "rejected".into(),
                    mark_reviewed: true,
                    rejection_reasons: Some(outcome.rejection_reasons),
                    ..Default::default()
                },
            )
            .await?;
            audit_transition(&mut tx, session, &app.id, "in_review", "rejected").await?;
        }
        tx.commit().await?;
    }

    load_owned(state, session, None).await
}

fn parse_uuid_or_not_found(value: &str) -> ApiResult<Uuid> {
    Uuid::parse_str(value).map_err(|_| ApiError::not_found("Document not found."))
}

async fn audit_transition(
    tx: &mut sqlx::PgConnection,
    session: &Session,
    app_id: &str,
    from: &str,
    to: &str,
) -> ApiResult<()> {
    write_audit(
        tx,
        AuditEntry {
            actor_id: Some(session.user_id),
            actor_role: Some(session.role_str()),
            action: "onboarding.state_change",
            entity_type: "onboarding_application",
            entity_id: app_id.to_string(),
            before: Some(json!({ "status": from })),
            after: Some(json!({ "status": to })),
        },
    )
    .await?;
    Ok(())
}

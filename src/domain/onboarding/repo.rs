use crate::contract::onboarding::{
    ApprovedAccountSummary, BusinessDetails, DirectorOrBeneficialOwner, OnboardingApplication,
    PrincipalRole, RejectionDetail, UploadedDocument,
};
use crate::error::ApiResult;
use crate::util::{iso, iso_opt};
use chrono::{DateTime, Utc};
use sqlx::types::Json;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct AppRow {
    id: Uuid,
    customer_id: Uuid,
    status: String,
    business: Option<Json<BusinessDetails>>,
    rejection_reasons: Option<Json<Vec<RejectionDetail>>>,
    approved_summary: Option<Json<ApprovedAccountSummary>>,
    submitted_at: Option<DateTime<Utc>>,
    reviewed_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct PrincipalRow {
    id: Uuid,
    full_name: String,
    role: String,
    ownership_percentage: Option<f64>,
    date_of_birth: Option<String>,
    bvn: Option<String>,
    nin: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DocRow {
    id: Uuid,
    #[sqlx(rename = "type")]
    doc_type: String,
    file_name: String,
    mime_type: String,
    size_bytes: i64,
    status: String,
    upload_progress_percent: i32,
    uploaded_at: Option<DateTime<Utc>>,
    error_message: Option<String>,
}

impl DocRow {
    fn into_contract(self) -> UploadedDocument {
        UploadedDocument {
            id: self.id.to_string(),
            r#type: self.doc_type,
            file_name: self.file_name,
            mime_type: self.mime_type,
            size_bytes: self.size_bytes,
            status: self.status,
            upload_progress_percent: self.upload_progress_percent,
            uploaded_at: iso_opt(self.uploaded_at),
            error_message: self.error_message,
        }
    }
}

const APP_COLS: &str = "id, customer_id, status, business, rejection_reasons,
                        approved_summary, submitted_at, reviewed_at";

async fn assemble(pool: &PgPool, app: AppRow) -> ApiResult<OnboardingApplication> {
    let principals: Vec<PrincipalRow> = sqlx::query_as(
        "select id, full_name, role, ownership_percentage,
                date_of_birth::text as date_of_birth, bvn, nin
           from onboarding_principals where application_id = $1 order by position",
    )
    .bind(app.id)
    .fetch_all(pool)
    .await?;

    let documents: Vec<DocRow> = sqlx::query_as(
        "select id, type, file_name, mime_type, size_bytes, status,
                upload_progress_percent, uploaded_at, error_message
           from onboarding_documents where application_id = $1 order by created_at",
    )
    .bind(app.id)
    .fetch_all(pool)
    .await?;

    Ok(OnboardingApplication {
        id: app.id.to_string(),
        customer_id: app.customer_id.to_string(),
        status: app.status,
        business: app.business.map(|j| j.0),
        principals: principals
            .into_iter()
            .map(|p| DirectorOrBeneficialOwner {
                id: Some(p.id.to_string()),
                full_name: p.full_name,
                role: PrincipalRole::parse(&p.role).unwrap_or(PrincipalRole::Director),
                ownership_percentage: p.ownership_percentage,
                date_of_birth: p.date_of_birth,
                bvn: p.bvn,
                nin: p.nin,
            })
            .collect(),
        documents: documents.into_iter().map(DocRow::into_contract).collect(),
        rejection_reasons: app.rejection_reasons.map(|j| j.0),
        approved_summary: app.approved_summary.map(|j| j.0),
        submitted_at: app.submitted_at.map(iso),
        reviewed_at: app.reviewed_at.map(iso),
    })
}

pub async fn find_by_customer(
    pool: &PgPool,
    customer_id: Uuid,
) -> ApiResult<Option<OnboardingApplication>> {
    let row: Option<AppRow> = sqlx::query_as(&format!(
        "select {APP_COLS} from onboarding_applications where customer_id = $1"
    ))
    .bind(customer_id)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok(Some(assemble(pool, r).await?)),
        None => Ok(None),
    }
}

pub async fn save_business(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    business: &BusinessDetails,
) -> ApiResult<()> {
    sqlx::query(
        "update onboarding_applications
            set business = $2, updated_at = now()
          where id = $1",
    )
    .bind(app_id)
    .bind(Json(business))
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn replace_principals(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    principals: &[DirectorOrBeneficialOwner],
) -> ApiResult<()> {
    sqlx::query("delete from onboarding_principals where application_id = $1")
        .bind(app_id)
        .execute(&mut *conn)
        .await?;
    for (i, p) in principals.iter().enumerate() {
        sqlx::query(
            "insert into onboarding_principals
               (application_id, position, full_name, role, ownership_percentage, date_of_birth, bvn, nin)
             values ($1, $2, $3, $4, $5, $6::date, $7, $8)",
        )
        .bind(app_id)
        .bind(i as i32)
        .bind(&p.full_name)
        .bind(p.role.as_str())
        .bind(p.ownership_percentage)
        .bind(p.date_of_birth.as_deref())
        .bind(p.bvn.as_deref())
        .bind(p.nin.as_deref())
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query("update onboarding_applications set updated_at = now() where id = $1")
        .bind(app_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub struct DocInput<'a> {
    pub doc_type: &'a str,
    pub file_name: &'a str,
    pub mime_type: &'a str,
    pub size_bytes: i64,
    pub storage_key: &'a str,
}

pub async fn upsert_document(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    input: DocInput<'_>,
) -> ApiResult<UploadedDocument> {
    let row: DocRow = sqlx::query_as(
        "insert into onboarding_documents
           (application_id, type, file_name, mime_type, size_bytes, status,
            upload_progress_percent, storage_key, uploaded_at)
         values ($1, $2, $3, $4, $5, 'uploaded', 100, $6, now())
         on conflict (application_id, type) do update
           set file_name = excluded.file_name, mime_type = excluded.mime_type,
               size_bytes = excluded.size_bytes, status = 'uploaded',
               upload_progress_percent = 100, storage_key = excluded.storage_key,
               uploaded_at = now(), error_message = null
         returning id, type, file_name, mime_type, size_bytes, status,
                   upload_progress_percent, uploaded_at, error_message",
    )
    .bind(app_id)
    .bind(input.doc_type)
    .bind(input.file_name)
    .bind(input.mime_type)
    .bind(input.size_bytes)
    .bind(input.storage_key)
    .fetch_one(conn)
    .await?;
    Ok(row.into_contract())
}

pub struct FoundDocument {
    pub document: UploadedDocument,
    pub application_id: Uuid,
    pub storage_key: Option<String>,
}

#[derive(sqlx::FromRow)]
struct DocWithMetaRow {
    id: Uuid,
    application_id: Uuid,
    #[sqlx(rename = "type")]
    doc_type: String,
    file_name: String,
    mime_type: String,
    size_bytes: i64,
    status: String,
    upload_progress_percent: i32,
    uploaded_at: Option<DateTime<Utc>>,
    error_message: Option<String>,
    storage_key: Option<String>,
}

pub async fn find_document(pool: &PgPool, doc_id: Uuid) -> ApiResult<Option<FoundDocument>> {
    let row: Option<DocWithMetaRow> = sqlx::query_as(
        "select id, application_id, type, file_name, mime_type, size_bytes, status,
                upload_progress_percent, uploaded_at, error_message, storage_key
           from onboarding_documents where id = $1",
    )
    .bind(doc_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| FoundDocument {
        document: UploadedDocument {
            id: r.id.to_string(),
            r#type: r.doc_type,
            file_name: r.file_name,
            mime_type: r.mime_type,
            size_bytes: r.size_bytes,
            status: r.status,
            upload_progress_percent: r.upload_progress_percent,
            uploaded_at: iso_opt(r.uploaded_at),
            error_message: r.error_message,
        },
        application_id: r.application_id,
        storage_key: r.storage_key,
    }))
}

pub async fn mark_document_uploaded(
    conn: &mut sqlx::PgConnection,
    doc_id: Uuid,
) -> ApiResult<UploadedDocument> {
    let row: DocRow = sqlx::query_as(
        "update onboarding_documents
            set status = 'uploaded', upload_progress_percent = 100,
                error_message = null, uploaded_at = now()
          where id = $1
        returning id, type, file_name, mime_type, size_bytes, status,
                  upload_progress_percent, uploaded_at, error_message",
    )
    .bind(doc_id)
    .fetch_one(conn)
    .await?;
    Ok(row.into_contract())
}

pub async fn delete_document(conn: &mut sqlx::PgConnection, doc_id: Uuid) -> ApiResult<()> {
    sqlx::query("delete from onboarding_documents where id = $1")
        .bind(doc_id)
        .execute(conn)
        .await?;
    Ok(())
}

#[derive(Default)]
pub struct StatusPatch {
    pub status: String,
    pub mark_submitted: bool,
    pub mark_reviewed: bool,
    pub approved_summary: Option<ApprovedAccountSummary>,
    pub rejection_reasons: Option<Vec<RejectionDetail>>,
}

/// Authoritative status write — approved_summary / rejection_reasons are set to
/// exactly what the patch carries (None => NULL), so resubmit after a rejection
/// clears stale reasons.
pub async fn patch_status(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    patch: StatusPatch,
) -> ApiResult<()> {
    sqlx::query(
        "update onboarding_applications
            set status = $2,
                submitted_at = case when $3 then now() else submitted_at end,
                reviewed_at  = case when $4 then now() else reviewed_at end,
                approved_summary  = $5,
                rejection_reasons = $6,
                updated_at = now()
          where id = $1",
    )
    .bind(app_id)
    .bind(&patch.status)
    .bind(patch.mark_submitted)
    .bind(patch.mark_reviewed)
    .bind(patch.approved_summary.map(Json))
    .bind(patch.rejection_reasons.filter(|r| !r.is_empty()).map(Json))
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn replace_kyb_checks(
    conn: &mut sqlx::PgConnection,
    app_id: Uuid,
    checks: &[super::kyb::CheckResult],
) -> ApiResult<()> {
    sqlx::query("delete from kyb_checks where application_id = $1")
        .bind(app_id)
        .execute(&mut *conn)
        .await?;
    for c in checks {
        sqlx::query(
            "insert into kyb_checks (application_id, check_key, passed, detail)
             values ($1, $2, $3, $4)",
        )
        .bind(app_id)
        .bind(c.key)
        .bind(c.passed)
        .bind(&c.detail)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

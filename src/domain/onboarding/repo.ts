import type { PoolClient } from 'pg';
import { query } from '../../db/pool';
import type {
  ApprovedAccountSummary,
  BusinessDetails,
  DirectorOrBeneficialOwner,
  OnboardingApplication,
  OnboardingStatus,
  RejectionDetail,
  UploadedDocument,
} from '../../contract';
import type { PrincipalInput } from './schemas';

interface ApplicationRow {
  id: string;
  customer_id: string;
  status: OnboardingStatus;
  business: BusinessDetails | null;
  rejection_reasons: RejectionDetail[] | null;
  approved_summary: ApprovedAccountSummary | null;
  submitted_at: Date | null;
  reviewed_at: Date | null;
}

interface PrincipalRow {
  id: string;
  full_name: string;
  role: DirectorOrBeneficialOwner['role'];
  ownership_percentage: string | null;
  date_of_birth: string | null;
  bvn: string | null;
  nin: string | null;
}

interface DocumentRow {
  id: string;
  type: UploadedDocument['type'];
  file_name: string;
  mime_type: string;
  size_bytes: string;
  status: UploadedDocument['status'];
  upload_progress_percent: number;
  uploaded_at: Date | null;
  error_message: string | null;
}

function iso(value: Date | null): string | undefined {
  return value ? value.toISOString() : undefined;
}

function toPrincipal(row: PrincipalRow): DirectorOrBeneficialOwner {
  return {
    id: row.id,
    fullName: row.full_name,
    role: row.role,
    ...(row.ownership_percentage !== null
      ? { ownershipPercentage: Number(row.ownership_percentage) }
      : {}),
    ...(row.date_of_birth !== null ? { dateOfBirth: row.date_of_birth } : {}),
    ...(row.bvn !== null ? { bvn: row.bvn } : {}),
    ...(row.nin !== null ? { nin: row.nin } : {}),
  };
}

function toDocument(row: DocumentRow): UploadedDocument {
  return {
    id: row.id,
    type: row.type,
    fileName: row.file_name,
    mimeType: row.mime_type,
    sizeBytes: Number(row.size_bytes),
    status: row.status,
    uploadProgressPercent: row.upload_progress_percent,
    ...(row.uploaded_at ? { uploadedAt: row.uploaded_at.toISOString() } : {}),
    ...(row.error_message !== null ? { errorMessage: row.error_message } : {}),
  };
}

// Any pg querier — the pool (module `query`) or a transaction client. Reads
// inside a transaction MUST use the client so they see uncommitted rows.
type Querier = <T>(text: string, params?: readonly unknown[]) => Promise<{ rows: T[] }>;

const poolQuerier: Querier = (text, params) =>
  query(text, params) as unknown as Promise<{ rows: never[] }>;

async function assembleFrom(
  appRow: ApplicationRow,
  q: Querier = poolQuerier,
): Promise<OnboardingApplication> {
  const [principals, documents] = await Promise.all([
    q<PrincipalRow>(
      `select id, full_name, role, ownership_percentage,
              date_of_birth::text as date_of_birth, bvn, nin
         from onboarding_principals
        where application_id = $1
        order by position`,
      [appRow.id],
    ),
    q<DocumentRow>(
      `select id, type, file_name, mime_type, size_bytes, status,
              upload_progress_percent, uploaded_at, error_message
         from onboarding_documents
        where application_id = $1
        order by created_at`,
      [appRow.id],
    ),
  ]);

  return {
    id: appRow.id,
    customerId: appRow.customer_id,
    status: appRow.status,
    business: appRow.business,
    principals: principals.rows.map(toPrincipal),
    documents: documents.rows.map(toDocument),
    ...(appRow.rejection_reasons ? { rejectionReasons: appRow.rejection_reasons } : {}),
    ...(appRow.approved_summary ? { approvedSummary: appRow.approved_summary } : {}),
    ...(iso(appRow.submitted_at) ? { submittedAt: iso(appRow.submitted_at) } : {}),
    ...(iso(appRow.reviewed_at) ? { reviewedAt: iso(appRow.reviewed_at) } : {}),
  };
}

const APP_COLUMNS = `id, customer_id, status, business, rejection_reasons,
                     approved_summary, submitted_at, reviewed_at`;

export async function findApplicationByCustomer(
  customerId: string,
): Promise<OnboardingApplication | null> {
  const { rows } = await query<ApplicationRow>(
    `select ${APP_COLUMNS} from onboarding_applications where customer_id = $1`,
    [customerId],
  );
  return rows[0] ? assembleFrom(rows[0]) : null;
}

export async function findApplicationById(id: string): Promise<OnboardingApplication | null> {
  const { rows } = await query<ApplicationRow>(
    `select ${APP_COLUMNS} from onboarding_applications where id = $1`,
    [id],
  );
  return rows[0] ? assembleFrom(rows[0]) : null;
}

/** For the transaction path — re-reads within the caller's client. */
export async function reloadApplication(
  client: PoolClient,
  id: string,
): Promise<OnboardingApplication> {
  const { rows } = await client.query<ApplicationRow>(
    `select ${APP_COLUMNS} from onboarding_applications where id = $1`,
    [id],
  );
  const row = rows[0];
  if (!row) throw new Error(`application ${id} vanished mid-transaction`);
  const clientQuerier: Querier = (text, params) =>
    client.query(text, params as unknown[]) as unknown as Promise<{ rows: never[] }>;
  return assembleFrom(row, clientQuerier);
}

export async function saveBusiness(
  client: PoolClient,
  applicationId: string,
  business: BusinessDetails,
): Promise<void> {
  await client.query(
    `update onboarding_applications
        set business = $2::jsonb,
            status = case when status = 'draft' then 'draft' else status end,
            updated_at = now()
      where id = $1`,
    [applicationId, JSON.stringify(business)],
  );
}

export async function replacePrincipals(
  client: PoolClient,
  applicationId: string,
  principals: readonly PrincipalInput[],
): Promise<void> {
  await client.query(`delete from onboarding_principals where application_id = $1`, [applicationId]);
  for (let i = 0; i < principals.length; i++) {
    const p = principals[i]!;
    await client.query(
      `insert into onboarding_principals
         (application_id, position, full_name, role, ownership_percentage, date_of_birth, bvn, nin)
       values ($1, $2, $3, $4, $5, $6, $7, $8)`,
      [
        applicationId,
        i,
        p.fullName,
        p.role,
        p.ownershipPercentage ?? null,
        p.dateOfBirth ?? null,
        p.bvn ?? null,
        p.nin ?? null,
      ],
    );
  }
  await client.query(
    `update onboarding_applications set updated_at = now() where id = $1`,
    [applicationId],
  );
}

export interface DocumentRecordInput {
  type: UploadedDocument['type'];
  fileName: string;
  mimeType: string;
  sizeBytes: number;
  storageKey: string;
}

export async function upsertUploadedDocument(
  client: PoolClient,
  applicationId: string,
  input: DocumentRecordInput,
): Promise<UploadedDocument> {
  const { rows } = await client.query<DocumentRow>(
    `insert into onboarding_documents
       (application_id, type, file_name, mime_type, size_bytes, status,
        upload_progress_percent, storage_key, uploaded_at)
     values ($1, $2, $3, $4, $5, 'uploaded', 100, $6, now())
     on conflict (application_id, type) do update
       set file_name = excluded.file_name,
           mime_type = excluded.mime_type,
           size_bytes = excluded.size_bytes,
           status = 'uploaded',
           upload_progress_percent = 100,
           storage_key = excluded.storage_key,
           uploaded_at = now(),
           error_message = null
     returning id, type, file_name, mime_type, size_bytes, status,
               upload_progress_percent, uploaded_at, error_message`,
    [applicationId, input.type, input.fileName, input.mimeType, input.sizeBytes, input.storageKey],
  );
  return toDocument(rows[0]!);
}

interface DocumentWithKeyRow extends DocumentRow {
  storage_key: string | null;
  application_id: string;
}

export async function findDocument(
  documentId: string,
): Promise<{ document: UploadedDocument; applicationId: string; storageKey: string | null } | null> {
  const { rows } = await query<DocumentWithKeyRow>(
    `select id, application_id, type, file_name, mime_type, size_bytes, status,
            upload_progress_percent, uploaded_at, error_message, storage_key
       from onboarding_documents
      where id = $1`,
    [documentId],
  );
  const row = rows[0];
  if (!row) return null;
  return { document: toDocument(row), applicationId: row.application_id, storageKey: row.storage_key };
}

export async function markDocumentUploaded(
  client: PoolClient,
  documentId: string,
): Promise<UploadedDocument> {
  const { rows } = await client.query<DocumentRow>(
    `update onboarding_documents
        set status = 'uploaded', upload_progress_percent = 100,
            error_message = null, uploaded_at = now()
      where id = $1
     returning id, type, file_name, mime_type, size_bytes, status,
               upload_progress_percent, uploaded_at, error_message`,
    [documentId],
  );
  return toDocument(rows[0]!);
}

export async function deleteDocument(client: PoolClient, documentId: string): Promise<void> {
  await client.query(`delete from onboarding_documents where id = $1`, [documentId]);
}

export interface StatusPatch {
  status: OnboardingStatus;
  submittedAt?: boolean;
  reviewedAt?: boolean;
  /** Always written: pass the summary to set it, omit to clear it. */
  approvedSummary?: ApprovedAccountSummary;
  /** Always written: pass reasons to set them, omit to clear them. */
  rejectionReasons?: readonly RejectionDetail[];
}

/**
 * Authoritative status write. `approved_summary` and `rejection_reasons` are
 * set to exactly what the patch carries (omitted => NULL), so a resubmit after
 * a rejection clears the stale reasons and vice versa. Only `submit` calls
 * this, and it always passes the complete intended picture.
 */
export async function patchStatus(
  client: PoolClient,
  applicationId: string,
  patch: StatusPatch,
): Promise<void> {
  await client.query(
    `update onboarding_applications
        set status = $2,
            submitted_at = case when $3::boolean then now() else submitted_at end,
            reviewed_at = case when $4::boolean then now() else reviewed_at end,
            approved_summary = $5::jsonb,
            rejection_reasons = $6::jsonb,
            updated_at = now()
      where id = $1`,
    [
      applicationId,
      patch.status,
      patch.submittedAt ?? false,
      patch.reviewedAt ?? false,
      patch.approvedSummary ? JSON.stringify(patch.approvedSummary) : null,
      patch.rejectionReasons && patch.rejectionReasons.length
        ? JSON.stringify(patch.rejectionReasons)
        : null,
    ],
  );
}

export async function replaceKybChecks(
  client: PoolClient,
  applicationId: string,
  checks: ReadonlyArray<{ key: string; passed: boolean; detail?: string }>,
): Promise<void> {
  await client.query(`delete from kyb_checks where application_id = $1`, [applicationId]);
  for (const check of checks) {
    await client.query(
      `insert into kyb_checks (application_id, check_key, passed, detail)
       values ($1, $2, $3, $4)`,
      [applicationId, check.key, check.passed, check.detail ?? null],
    );
  }
}

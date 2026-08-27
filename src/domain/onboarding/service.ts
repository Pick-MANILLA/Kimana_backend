import type { PoolClient } from 'pg';
import { withTx } from '../../db/pool';
import { ApiError } from '../../errors';
import { writeAudit } from '../../audit/writeAudit';
import { documentStore } from '../../storage';
import type {
  ApprovedAccountSummary,
  BusinessDetails,
  IndustrySector,
  OnboardingApplication,
  UploadedDocument,
} from '../../contract';
import type { RequestSession } from '../../http/auth';
import type { BusinessDetailsInput, PrincipalInput } from './schemas';
import * as repo from './repo';
import { kybProvider } from './kyb';

const ALLOWED_MIME_TYPES = new Set(['application/pdf', 'image/jpeg', 'image/png']);
const MAX_FILE_SIZE_BYTES = 10 * 1024 * 1024;

// Lifted from Kimana_frontend/src/api/mock/onboardingApi.ts.
const INDUSTRY_SEGMENT_LABEL: Record<IndustrySector, string> = {
  agriculture_agro_export: 'Agro Exporter',
  textiles_apparel: 'Textiles Exporter',
  solid_minerals: 'Solid Minerals Exporter',
  manufacturing: 'Manufacturing Exporter',
  oil_gas_services: 'Oil & Gas Services',
  technology: 'Technology Exporter',
  trading_commodities: 'Commodities Trader',
  other: 'Trading Business',
};

function generateAccountId(legalName: string): string {
  const initials = legalName
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 3)
    .map((word) => word[0]?.toUpperCase() ?? '')
    .join('');
  const serial = Math.floor(Math.random() * 90000 + 10000);
  return `${initials || 'KMA'}-${serial}`;
}

/**
 * Loads the caller's application and, if `applicationId` was supplied, checks
 * it matches — mirroring the mock's `NOT_FOUND` on a wrong id.
 */
async function loadOwnedApplication(
  session: RequestSession,
  applicationId?: string,
): Promise<OnboardingApplication> {
  const application = await repo.findApplicationByCustomer(session.customerId);
  if (!application) throw ApiError.notFound('No onboarding application for this customer.');
  if (applicationId && applicationId !== application.id) {
    throw ApiError.notFound('Application not found.');
  }
  return application;
}

export async function getApplication(session: RequestSession): Promise<OnboardingApplication> {
  return loadOwnedApplication(session);
}

export async function saveBusinessDetails(
  session: RequestSession,
  business: BusinessDetailsInput,
  applicationId?: string,
): Promise<OnboardingApplication> {
  const application = await loadOwnedApplication(session, applicationId);

  return withTx(async (client) => {
    await repo.saveBusiness(client, application.id, business as BusinessDetails);
    await writeAudit(client, {
      actorId: session.userId,
      actorRole: session.role,
      action: 'onboarding.business_saved',
      entityType: 'onboarding_application',
      entityId: application.id,
      before: application.business,
      after: business,
    });
    return repo.reloadApplication(client, application.id);
  });
}

export async function savePrincipals(
  session: RequestSession,
  principals: readonly PrincipalInput[],
  applicationId?: string,
): Promise<OnboardingApplication> {
  const application = await loadOwnedApplication(session, applicationId);

  return withTx(async (client) => {
    await repo.replacePrincipals(client, application.id, principals);
    await writeAudit(client, {
      actorId: session.userId,
      actorRole: session.role,
      action: 'onboarding.principals_saved',
      entityType: 'onboarding_application',
      entityId: application.id,
      before: application.principals,
      after: principals,
    });
    return repo.reloadApplication(client, application.id);
  });
}

export interface DocumentUpload {
  type: UploadedDocument['type'];
  fileName: string;
  mimeType: string;
  buffer: Buffer;
}

function assertFile(mimeType: string, sizeBytes: number): void {
  if (!ALLOWED_MIME_TYPES.has(mimeType)) {
    throw ApiError.validation('That file type isn’t supported. Upload a PDF, JPG, or PNG.');
  }
  if (sizeBytes > MAX_FILE_SIZE_BYTES) {
    throw ApiError.validation('That file is larger than 10 MB. Compress it or choose a smaller copy.');
  }
}

export async function uploadDocument(
  session: RequestSession,
  upload: DocumentUpload,
  applicationId?: string,
): Promise<UploadedDocument> {
  const application = await loadOwnedApplication(session, applicationId);
  assertFile(upload.mimeType, upload.buffer.byteLength);

  const storageKey = `onboarding/${application.id}/${upload.type}/${Date.now()}-${upload.fileName}`;
  await documentStore.put(storageKey, upload.buffer, upload.mimeType);

  const previous = application.documents.find((d) => d.type === upload.type);

  const document = await withTx(async (client) => {
    const saved = await repo.upsertUploadedDocument(client, application.id, {
      type: upload.type,
      fileName: upload.fileName,
      mimeType: upload.mimeType,
      sizeBytes: upload.buffer.byteLength,
      storageKey,
    });
    await writeAudit(client, {
      actorId: session.userId,
      actorRole: session.role,
      action: previous ? 'onboarding.document_replaced' : 'onboarding.document_uploaded',
      entityType: 'onboarding_document',
      entityId: saved.id,
      before: previous ?? null,
      after: saved,
    });
    return saved;
  });

  return document;
}

export async function retryDocumentUpload(
  session: RequestSession,
  documentId: string,
  applicationId?: string,
): Promise<UploadedDocument> {
  const application = await loadOwnedApplication(session, applicationId);
  const found = await repo.findDocument(documentId);
  if (!found || found.applicationId !== application.id) {
    throw ApiError.notFound('Document not found.');
  }

  // Slice: bytes are already stored on the original upload, so retry just
  // clears the failed state. A real retry re-accepts the file.
  return withTx(async (client) => {
    const updated = await repo.markDocumentUploaded(client, documentId);
    await writeAudit(client, {
      actorId: session.userId,
      actorRole: session.role,
      action: 'onboarding.document_retried',
      entityType: 'onboarding_document',
      entityId: documentId,
      before: found.document,
      after: updated,
    });
    return updated;
  });
}

export async function removeDocument(
  session: RequestSession,
  documentId: string,
  applicationId?: string,
): Promise<void> {
  const application = await loadOwnedApplication(session, applicationId);
  const found = await repo.findDocument(documentId);
  if (!found || found.applicationId !== application.id) {
    throw ApiError.notFound('Document not found.');
  }

  await withTx(async (client) => {
    await repo.deleteDocument(client, documentId);
    await writeAudit(client, {
      actorId: session.userId,
      actorRole: session.role,
      action: 'onboarding.document_removed',
      entityType: 'onboarding_document',
      entityId: documentId,
      before: found.document,
      after: null,
    });
  });

  if (found.storageKey) {
    await documentStore.delete(found.storageKey).catch(() => undefined);
  }
}

/**
 * Slice: synchronous KYB that always approves. Walks the real status chain
 * (draft -> submitted -> in_review -> approved), writing an audit row per
 * transition, runs the KYB provider, then lands on `approved` (with
 * approvedSummary) or `rejected` (with rejectionReasons). The stub provider
 * always approves unless the data trips a documented trigger — see
 * kyb/stubProvider.ts.
 *
 * The HTTP request stays open until a terminal status is reached, matching the
 * contract ("resolves once checks complete with the final status") and the
 * frontend, which does not poll. A real provider that runs for minutes would
 * want this split into submit + webhook/poll — see docs/backend-plan.md §03.
 */
async function auditStateChange(
  client: PoolClient,
  session: RequestSession,
  applicationId: string,
  from: string,
  to: string,
): Promise<void> {
  await writeAudit(client, {
    actorId: session.userId,
    actorRole: session.role,
    action: 'onboarding.state_change',
    entityType: 'onboarding_application',
    entityId: applicationId,
    before: { status: from },
    after: { status: to },
  });
}

export async function submit(
  session: RequestSession,
  applicationId?: string,
): Promise<OnboardingApplication> {
  const application = await loadOwnedApplication(session, applicationId);

  if (!application.business) {
    throw ApiError.validation('Add your business details before submitting.');
  }
  if (application.status === 'submitted' || application.status === 'in_review') {
    throw ApiError.conflict('This application is already being reviewed.');
  }

  // draft -> submitted -> in_review, committed before the checks run so a
  // concurrent getApplication sees in_review.
  await withTx(async (client) => {
    await repo.patchStatus(client, application.id, { status: 'submitted', submittedAt: true });
    await auditStateChange(client, session, application.id, application.status, 'submitted');
    await repo.patchStatus(client, application.id, { status: 'in_review' });
    await auditStateChange(client, session, application.id, 'submitted', 'in_review');
  });

  const outcome = await kybProvider.runChecks(application);

  return withTx(async (client) => {
    await repo.replaceKybChecks(client, application.id, outcome.checks);

    if (outcome.approved) {
      const business = application.business!;
      const summary: ApprovedAccountSummary = {
        accountId: generateAccountId(business.legalName),
        riskRatingLabel: 'Medium-Low',
        segment: INDUSTRY_SEGMENT_LABEL[business.industry] ?? 'Trading Business',
        corridor: 'NGN → USD / EUR',
        monthlyLimit: { amountMinor: 100_000_00, currency: 'USD' },
      };
      await repo.patchStatus(client, application.id, {
        status: 'approved',
        reviewedAt: true,
        approvedSummary: summary,
      });
      await auditStateChange(client, session, application.id, 'in_review', 'approved');
    } else {
      await repo.patchStatus(client, application.id, {
        status: 'rejected',
        reviewedAt: true,
        rejectionReasons: outcome.rejectionReasons,
      });
      await auditStateChange(client, session, application.id, 'in_review', 'rejected');
    }

    return repo.reloadApplication(client, application.id);
  });
}

// ============================================================================
//  DROP-IN FOR THE FRONTEND — this file belongs in Kimana_frontend, not here.
//
//  Place at:  Kimana_frontend/src/api/live/client.ts
//  Then edit  Kimana_frontend/src/api/index.ts  (see integration/README.md).
//
//  It implements the P1 + P2 endpoints (auth, onboarding, dashboard, fx,
//  recipients, quote, transfers) against the real backend over HTTP, and
//  delegates every not-yet-built method (trade documents, screening, delays,
//  all ops) to the existing mock. As each later phase ships, move those methods
//  off `mock` and onto `http`.
//
//  The backend is Rust (axum), but the wire contract is plain HTTP/JSON — this
//  client is unchanged by the language.
// ============================================================================

import type { ApiClient } from '../contract';
import type {
  BusinessDetails,
  CurrencyCode,
  DirectorOrBeneficialOwner,
  DocumentFileInput,
  FirmQuote,
  Id,
  IndicativeRate,
  NewRecipientInput,
  OnboardingApplication,
  Recipient,
  RequestFirmQuoteInput,
  Transfer,
  TransferStatus,
  TransferTimeline,
  UploadedDocument,
} from '../types';
import type { CreateTransferInput } from '../types/transfer';
import { mockApiClient } from '../mock';

const BASE_URL =
  (import.meta.env.VITE_API_URL as string | undefined)?.replace(/\/$/, '') ??
  'http://localhost:4000';

interface WireError {
  code: string;
  message: string;
  retryable: boolean;
}

class HttpApiError extends Error {
  readonly code: string;
  readonly retryable: boolean;
  constructor(body: WireError) {
    super(body.message);
    this.name = 'ApiError';
    this.code = body.code;
    this.retryable = body.retryable;
  }
}

async function parseOrThrow(res: Response): Promise<unknown> {
  if (res.status === 204) return undefined;
  const text = await res.text();
  const json = text ? JSON.parse(text) : undefined;
  if (!res.ok) {
    if (json && typeof json === 'object' && 'code' in json) {
      throw new HttpApiError(json as WireError);
    }
    throw new HttpApiError({
      code: 'SERVER_ERROR',
      message: `Request failed (${res.status}).`,
      retryable: res.status >= 500,
    });
  }
  return json;
}

async function http<T>(
  method: string,
  path: string,
  body?: unknown,
  extraHeaders?: Record<string, string>,
): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${BASE_URL}${path}`, {
      method,
      credentials: 'include',
      headers: {
        ...(body === undefined ? {} : { 'content-type': 'application/json' }),
        ...extraHeaders,
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new HttpApiError({
      code: 'NETWORK',
      message: 'The connection dropped before this finished. Check your signal and try again.',
      retryable: true,
    });
  }
  return (await parseOrThrow(res)) as T;
}

async function upload(
  path: string,
  fields: Record<string, string>,
  file: DocumentFileInput,
): Promise<UploadedDocument> {
  const form = new FormData();
  for (const [key, value] of Object.entries(fields)) form.append(key, value);
  form.append('file', file.data, file.fileName);

  let res: Response;
  try {
    res = await fetch(`${BASE_URL}${path}`, { method: 'POST', credentials: 'include', body: form });
  } catch {
    throw new HttpApiError({
      code: 'NETWORK',
      message: 'The upload was interrupted. Try again.',
      retryable: true,
    });
  }
  return (await parseOrThrow(res)) as UploadedDocument;
}

export function createLiveApiClient(): ApiClient {
  return {
    ...mockApiClient,

    auth: {
      getSession: () => http('GET', '/session'),
    },

    onboarding: {
      getApplication: (_customerId: Id) =>
        http<OnboardingApplication>('GET', '/onboarding/application'),

      saveBusinessDetails: (applicationId: Id, business: BusinessDetails) =>
        http<OnboardingApplication>('PUT', '/onboarding/application/business', {
          applicationId,
          business,
        }),

      savePrincipals: (applicationId: Id, principals: readonly DirectorOrBeneficialOwner[]) =>
        http<OnboardingApplication>('PUT', '/onboarding/application/principals', {
          applicationId,
          principals,
        }),

      uploadDocument: async (
        applicationId: Id,
        file: DocumentFileInput,
        onProgress?: (percent: number) => void,
      ) => {
        const doc = await upload(
          '/onboarding/application/documents',
          { applicationId, type: file.type },
          file,
        );
        onProgress?.(100);
        return doc;
      },

      retryDocumentUpload: (_applicationId: Id, documentId: Id) =>
        http<UploadedDocument>(
          'POST',
          `/onboarding/application/documents/${encodeURIComponent(documentId)}/retry`,
        ),

      removeDocument: (_applicationId: Id, documentId: Id) =>
        http<void>(
          'DELETE',
          `/onboarding/application/documents/${encodeURIComponent(documentId)}`,
        ),

      submit: (applicationId: Id) =>
        http<OnboardingApplication>('POST', '/onboarding/application/submit', { applicationId }),
    },

    dashboard: {
      getOverview: (_customerId: Id) => http('GET', '/dashboard/overview'),
    },

    quote: {
      getIndicativeRate: (sendCurrency: CurrencyCode, receiveCurrency: CurrencyCode) =>
        http<IndicativeRate>(
          'GET',
          `/rates/indicative?send=${sendCurrency}&receive=${receiveCurrency}`,
        ),
      requestFirmQuote: (input: RequestFirmQuoteInput) =>
        http<FirmQuote>('POST', '/quotes', input),
    },

    recipients: {
      listRecipients: (_customerId: Id) => http<readonly Recipient[]>('GET', '/recipients'),
      validateBankAccount: (input: NewRecipientInput) =>
        http<{ accountName: string }>('POST', '/recipients/validate', input),
      saveRecipient: (input: NewRecipientInput & { accountName: string }) =>
        http<Recipient>('POST', '/recipients', input),
    },

    transfers: {
      createTransfer: (input: CreateTransferInput) =>
        http<Transfer>('POST', '/transfers', input, {
          'idempotency-key': input.idempotencyKey,
        }),
      getTransfer: (id: Id) => http<Transfer>('GET', `/transfers/${encodeURIComponent(id)}`),
      getTimeline: (id: Id) =>
        http<TransferTimeline>('GET', `/transfers/${encodeURIComponent(id)}/timeline`),
      listTransfers: (_customerId: Id, filter?: { status?: TransferStatus }) =>
        http<readonly Transfer[]>(
          'GET',
          `/transfers${filter?.status ? `?status=${filter.status}` : ''}`,
        ),
    },
  };
}

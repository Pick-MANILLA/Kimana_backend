// ============================================================================
//  DROP-IN FOR THE FRONTEND — this file belongs in Kimana_frontend, not here.
//
//  Place at:  Kimana_frontend/src/api/live/client.ts
//  Then edit  Kimana_frontend/src/api/index.ts  (see integration/README.md).
//
//  It implements the P1 slice endpoints (auth, onboarding, dashboard) against
//  the real backend over HTTP, and delegates every not-yet-built method to the
//  existing mock. As each later phase ships, move those methods off `mock` and
//  onto `http`.
// ============================================================================

import type { ApiClient } from '../contract';
import type {
  BusinessDetails,
  DirectorOrBeneficialOwner,
  DocumentFileInput,
  Id,
  OnboardingApplication,
  UploadedDocument,
} from '../types';
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

async function http<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${BASE_URL}${path}`, {
      method,
      credentials: 'include',
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
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
  };
}

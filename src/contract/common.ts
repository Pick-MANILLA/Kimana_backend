// Shared primitives referenced across every domain module.

export type Id = string;

/** ISO-8601 UTC timestamp, e.g. "2026-08-27T09:15:00Z". */
export type ISODateTime = string;

/** ISO date only, e.g. "2026-08-27". */
export type ISODate = string;

export type CurrencyCode = 'NGN' | 'KES' | 'GHS' | 'ZAR' | 'XOF' | 'XAF' | 'EGP' | 'USD' | 'EUR' | 'GBP';

/**
 * Signed integer minor units + currency code. The only representation of
 * money anywhere in this codebase — never a bare number, never a float.
 * Sign convention (e.g. debit/credit) is defined per field, not by the type.
 */
export interface Money {
  readonly amountMinor: number;
  readonly currency: CurrencyCode;
}

export interface Paginated<T> {
  readonly items: readonly T[];
  readonly nextCursor?: string;
  readonly total?: number;
}

export type ApiErrorCode =
  | 'NETWORK'
  | 'TIMEOUT'
  | 'VALIDATION'
  | 'NOT_FOUND'
  | 'CONFLICT'
  | 'UNAUTHORIZED'
  | 'FORBIDDEN'
  | 'COMPLIANCE_HOLD'
  | 'PARTNER_FAILURE'
  | 'RATE_EXPIRED'
  | 'SERVER_ERROR';

/**
 * Thrown (not returned) by every ApiClient method on failure, so callers
 * work naturally with TanStack Query's error channel.
 */
export interface ApiError extends Error {
  readonly code: ApiErrorCode;
  readonly retryable: boolean;
}

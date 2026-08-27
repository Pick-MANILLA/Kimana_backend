import type { ApiErrorCode } from './contract';

/** ApiErrorCode -> HTTP status. See docs/backend-plan.md §02. */
const HTTP_STATUS: Record<ApiErrorCode, number> = {
  NETWORK: 503,
  TIMEOUT: 504,
  VALIDATION: 400,
  NOT_FOUND: 404,
  CONFLICT: 409,
  UNAUTHORIZED: 401,
  FORBIDDEN: 403,
  COMPLIANCE_HOLD: 409,
  PARTNER_FAILURE: 502,
  RATE_EXPIRED: 409,
  SERVER_ERROR: 500,
};

const DEFAULT_RETRYABLE: Record<ApiErrorCode, boolean> = {
  NETWORK: true,
  TIMEOUT: true,
  VALIDATION: false,
  NOT_FOUND: false,
  CONFLICT: false,
  UNAUTHORIZED: false,
  FORBIDDEN: false,
  COMPLIANCE_HOLD: false,
  PARTNER_FAILURE: true,
  RATE_EXPIRED: true,
  SERVER_ERROR: true,
};

/**
 * Thrown by services and route handlers. The Fastify error handler turns it
 * into `{ code, message, retryable }` with the mapped status — the exact shape
 * the frontend's ApiError consumers expect.
 */
export class ApiError extends Error {
  readonly code: ApiErrorCode;
  readonly retryable: boolean;
  readonly httpStatus: number;

  constructor(code: ApiErrorCode, message: string, retryable: boolean = DEFAULT_RETRYABLE[code]) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.retryable = retryable;
    this.httpStatus = HTTP_STATUS[code];
  }

  static validation(message: string): ApiError {
    return new ApiError('VALIDATION', message, false);
  }
  static notFound(message = 'Not found.'): ApiError {
    return new ApiError('NOT_FOUND', message, false);
  }
  static conflict(message: string): ApiError {
    return new ApiError('CONFLICT', message, false);
  }
  static unauthorized(message = 'Not signed in.'): ApiError {
    return new ApiError('UNAUTHORIZED', message, false);
  }
  static forbidden(message = 'Not allowed.'): ApiError {
    return new ApiError('FORBIDDEN', message, false);
  }
}

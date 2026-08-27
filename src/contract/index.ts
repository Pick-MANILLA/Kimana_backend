// Mirror of Kimana_frontend/src/api/types/index.ts — see ./README.md.
// The server imports contract shapes from here (`../contract`), never from the
// individual files, so the promotion to an `@kimana/contract` package is a
// single-path change.

export type * from './common';
export type * from './auth';
export type * from './dashboard';
export type * from './onboarding';
export type * from './screening';
export type * from './quote';
export type * from './transfer';
export type * from './ledger';
export type * from './documents';
export type * from './reconciliation';
export type * from './partners';
export type * from './audit';
export type * from './opsTransactions';

// The full server contract (both ends implement this identical interface).
export type { ApiClient } from './contract';

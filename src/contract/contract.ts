// Mirror of Kimana_frontend/src/api/contract.ts. The only local change from the
// source: import paths are `./x` instead of `./types/x`, because this vendored
// copy flattens the type files alongside this one. Shapes are untouched.

import type { AuthApi } from './auth';
import type { AuditApi } from './audit';
import type { DashboardApi } from './dashboard';
import type { TradeDocumentApi, OpsTradeDocumentApi } from './documents';
import type { LedgerApi, OpsLedgerApi } from './ledger';
import type { OnboardingApi } from './onboarding';
import type { OpsTransactionApi } from './opsTransactions';
import type { CustomerDelayApi, PartnerApi } from './partners';
import type { QuoteApi } from './quote';
import type { ReconciliationApi } from './reconciliation';
import type { ScreeningApi } from './screening';
import type { RecipientApi, TransferApi } from './transfer';

/**
 * The complete server contract. Both the mock layer (src/api/mock/) and the
 * eventual live client implement this same interface, so swapping one for
 * the other is a single module change wherever an ApiClient is constructed.
 */
export interface ApiClient {
  readonly auth: AuthApi;
  readonly dashboard: DashboardApi;
  readonly onboarding: OnboardingApi;
  readonly screening: ScreeningApi;
  readonly quote: QuoteApi;
  readonly recipients: RecipientApi;
  readonly transfers: TransferApi;
  readonly ledger: LedgerApi;
  readonly tradeDocuments: TradeDocumentApi;
  readonly reconciliation: ReconciliationApi;
  readonly partners: PartnerApi;
  readonly delays: CustomerDelayApi;
  readonly audit: AuditApi;

  // Ops-only surfaces. Bundled here rather than a separate interface so one
  // ApiClient type covers both areas — route-level code splitting (not this
  // contract) is what keeps ops code out of the customer bundle.
  readonly opsTransactions: OpsTransactionApi;
  readonly opsLedger: OpsLedgerApi;
  readonly opsTradeDocuments: OpsTradeDocumentApi;
}

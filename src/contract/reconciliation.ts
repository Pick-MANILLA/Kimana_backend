import type { CurrencyCode, Id, ISODate, ISODateTime, Money } from './common';

export type BreakStatus = 'open' | 'investigating' | 'resolved';

export interface PartnerLedgerComparison {
  readonly currency: CurrencyCode;
  readonly partnerBalance: Money;
  readonly ledgerBalance: Money;
  /** partnerBalance - ledgerBalance, signed. */
  readonly difference: Money;
  readonly asOf: ISODateTime;
}

export interface ReconciliationBreak {
  readonly id: Id;
  readonly currency: CurrencyCode;
  readonly amount: Money;
  readonly ageDays: number;
  readonly status: BreakStatus;
  readonly linkedTransferIds: readonly Id[];
  readonly openedAt: ISODateTime;
  /** Required before status can move to 'resolved'. */
  readonly resolutionNote?: string;
}

export interface ReconciliationApi {
  getDashboard(date: ISODate): Promise<readonly PartnerLedgerComparison[]>;
  listBreaks(filter?: { status?: BreakStatus }): Promise<readonly ReconciliationBreak[]>;
  getBreak(id: Id): Promise<ReconciliationBreak>;
  updateBreakStatus(id: Id, status: BreakStatus, resolutionNote?: string): Promise<ReconciliationBreak>;
}

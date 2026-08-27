import type { CurrencyCode, Id, Money } from './common';
import type { AccountBalance } from './ledger';

export type BalanceHighlightTone = 'success' | 'warning' | 'danger';

/** Small narrative line shown under a currency balance — pre-composed server-side. */
export interface BalanceHighlight {
  readonly currency: CurrencyCode;
  readonly secondaryLine?: string;
  readonly deltaText: string;
  readonly deltaTone: BalanceHighlightTone;
}

export interface DashboardStats {
  readonly volume30d: Money;
  readonly transfersInProgress: number;
  readonly payoutSuccessRatePercent: number;
  readonly avgSettlementSeconds: number;
}

/** Drives both the icon and the "N urgent" counter badge. */
export type PendingActionKind = 'action_required' | 'in_review' | 'submitted';

export interface PendingAction {
  readonly id: Id;
  readonly title: string;
  readonly subtitle: string;
  readonly kind: PendingActionKind;
  readonly transferId?: Id;
}

/**
 * Trade-finance teaser shown on the dashboard. Not part of the original P0
 * scope's twelve surfaces — a display-only widget, no application flow
 * modeled yet.
 */
export interface WorkingCapitalOffer {
  readonly maxAdvance: Money;
  readonly basisDescription: string;
  readonly monthlyRatePercent: number;
}

/**
 * One aggregate call for the dashboard home screen, composed server-side
 * from ledger/transfer/ops data the customer app doesn't otherwise need to
 * join client-side.
 */
export interface DashboardOverview {
  readonly displayName: string;
  readonly businessName: string;
  readonly accountId: string;
  readonly balances: readonly AccountBalance[];
  readonly balanceHighlights: readonly BalanceHighlight[];
  readonly stats: DashboardStats;
  readonly pendingActions: readonly PendingAction[];
  readonly workingCapitalOffer?: WorkingCapitalOffer;
}

export interface DashboardApi {
  getOverview(customerId: Id): Promise<DashboardOverview>;
}

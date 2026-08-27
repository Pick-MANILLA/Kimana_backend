import type { BalanceHighlight, PendingAction, WorkingCapitalOffer } from '../../contract';

/**
 * Dashboard content that has no real source in the P1 slice. Values match
 * Kimana_frontend/src/api/mock/{seed,dashboardApi}.ts so the screen looks
 * right. In P2 these become computed:
 *   - balanceHighlights: from FX + transfer activity
 *   - stats.volume30d / transfersInProgress: from the transfers table
 *   - pendingActions: from trade-document + screening state
 *   - workingCapitalOffer: still a display-only teaser
 */

export const balanceHighlights: readonly BalanceHighlight[] = [
  { currency: 'NGN', secondaryLine: '≈ USD 29,330', deltaText: '+₦2.4M this month', deltaTone: 'success' },
  { currency: 'USD', secondaryLine: 'Pending: +$18,500', deltaText: 'TXN-8843 settling', deltaTone: 'success' },
  { currency: 'EUR', secondaryLine: '≈ USD 19,760', deltaText: 'TXN-8842 in progress', deltaTone: 'warning' },
];

// volume30d and transfersInProgress are now computed from the transfers table;
// these two still have no source.
export const staticStats = {
  payoutSuccessRatePercent: 98.3,
  avgSettlementSeconds: 402,
};

export const pendingActions: readonly PendingAction[] = [
  {
    id: 'pact_paar_8842',
    title: 'PAAR — TXN-8842',
    subtitle: 'Upload required',
    kind: 'action_required',
    transferId: 'txn_8842',
  },
  {
    id: 'pact_formq_8843',
    title: 'Form Q — Sesame export',
    subtitle: 'Pending review',
    kind: 'in_review',
    transferId: 'txn_8843',
  },
  {
    id: 'pact_bol_8843',
    title: 'BoL — TXN-8843',
    subtitle: 'Submitted for verification',
    kind: 'submitted',
    transferId: 'txn_8843',
  },
];

export const workingCapitalOffer: WorkingCapitalOffer = {
  maxAdvance: { amountMinor: 3_825_000, currency: 'USD' },
  basisDescription: 'Against Amsterdam Commodities receivable',
  monthlyRatePercent: 2.5,
};

import type { DashboardOverview } from '../../contract';
import type { RequestSession } from '../../http/auth';
import { getBalances } from '../ledger/repo';
import { findApplicationByCustomer } from '../onboarding/repo';
import {
  balanceHighlights,
  pendingActions,
  stats,
  workingCapitalOffer,
} from './placeholderContent';

// Fallbacks match Kimana_frontend/src/api/mock/dashboardApi.ts.
const FALLBACK_BUSINESS_NAME = 'Adunola Exports Ltd';
const FALLBACK_ACCOUNT_ID = 'AEL-00029';

export async function getOverview(session: RequestSession): Promise<DashboardOverview> {
  const [balances, application] = await Promise.all([
    getBalances(session.customerId),
    findApplicationByCustomer(session.customerId),
  ]);

  const firstPrincipalName = application?.principals[0]?.fullName.split(/\s+/)[0];

  return {
    displayName: firstPrincipalName ?? session.displayName,
    businessName: application?.business?.legalName ?? FALLBACK_BUSINESS_NAME,
    accountId: application?.approvedSummary?.accountId ?? FALLBACK_ACCOUNT_ID,
    balances,
    balanceHighlights,
    stats,
    pendingActions,
    workingCapitalOffer,
  };
}

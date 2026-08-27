import type { CurrencyCode } from '../../contract';

export interface AccountLookup {
  readonly accountNumber: string;
  readonly bankCode: string;
  readonly currency: CurrencyCode;
  readonly country: string;
}

/**
 * Resolves the account holder name for a destination account, so the customer
 * can confirm before saving. The stub is deterministic; a real integration
 * calls the payout partner's name-lookup API.
 */
export interface BankAccountNameResolver {
  resolve(lookup: AccountLookup): Promise<{ accountName: string } | null>;
}

// Matches Kimana_frontend/src/api/mock/recipientApi.ts.
export const stubNameResolver: BankAccountNameResolver = {
  async resolve(lookup) {
    const tail = lookup.accountNumber.slice(-4);
    return { accountName: `Verified Beneficiary (${tail})` };
  },
};

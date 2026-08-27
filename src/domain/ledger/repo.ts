import { query } from '../../db/pool';
import type { AccountBalance, CurrencyCode } from '../../contract';

interface BalanceRow {
  account_id: string;
  currency: CurrencyCode;
  balance_minor: string;
}

/**
 * One AccountBalance per currency the customer holds, each the signed sum of
 * that account's ledger_entries. `pending` has no source until settlement
 * modelling exists in P2, so it is omitted.
 */
export async function getBalances(customerId: string): Promise<AccountBalance[]> {
  const { rows } = await query<BalanceRow>(
    `select a.id as account_id,
            a.currency as currency,
            coalesce(sum(le.amount_minor), 0)::bigint as balance_minor
       from accounts a
       left join ledger_entries le on le.account_id = a.id
      where a.customer_id = $1
      group by a.id, a.currency
      order by a.currency`,
    [customerId],
  );

  const asOf = new Date().toISOString();
  return rows.map((row) => ({
    accountId: row.account_id,
    currency: row.currency,
    balance: { amountMinor: Number(row.balance_minor), currency: row.currency },
    asOf,
  }));
}

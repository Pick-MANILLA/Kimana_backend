import type { PoolClient } from 'pg';
import type { CurrencyCode } from '../../contract';

/**
 * Returns the id of the customer's account in `currency`, creating a
 * zero-balance one (no opening entry) if they don't hold it yet.
 */
export async function getOrCreateAccount(
  client: PoolClient,
  customerId: string,
  currency: CurrencyCode,
): Promise<string> {
  const existing = await client.query<{ id: string }>(
    `select id from accounts where customer_id = $1 and currency = $2`,
    [customerId, currency],
  );
  if (existing.rows[0]) return existing.rows[0].id;

  const inserted = await client.query<{ id: string }>(
    `insert into accounts (customer_id, currency) values ($1, $2) returning id`,
    [customerId, currency],
  );
  return inserted.rows[0]!.id;
}

export interface LedgerPosting {
  accountId: string;
  transferId: string;
  /** Signed minor units: positive = credit, negative = debit. */
  amountMinor: number;
  currency: CurrencyCode;
  description: string;
  reversalOfEntryId?: string;
}

/**
 * Appends one ledger entry, computing running_balance_minor under an account
 * row lock so concurrent postings to the same account serialise. Returns the
 * new entry's id and the resulting balance.
 */
export async function postLedgerEntry(
  client: PoolClient,
  posting: LedgerPosting,
): Promise<{ entryId: string; runningBalanceMinor: number }> {
  await client.query(`select id from accounts where id = $1 for update`, [posting.accountId]);

  const { rows } = await client.query<{ balance: string }>(
    `select coalesce(sum(amount_minor), 0)::bigint as balance
       from ledger_entries where account_id = $1`,
    [posting.accountId],
  );
  const running = Number(rows[0]!.balance) + posting.amountMinor;

  const inserted = await client.query<{ id: string }>(
    `insert into ledger_entries
       (account_id, transfer_id, amount_minor, currency, running_balance_minor, description, reversal_of_entry_id)
     values ($1, $2, $3, $4, $5, $6, $7)
     returning id`,
    [
      posting.accountId,
      posting.transferId,
      posting.amountMinor,
      posting.currency,
      running,
      posting.description,
      posting.reversalOfEntryId ?? null,
    ],
  );

  return { entryId: inserted.rows[0]!.id, runningBalanceMinor: running };
}

export async function accountBalanceMinor(
  client: PoolClient,
  accountId: string,
): Promise<number> {
  const { rows } = await client.query<{ balance: string }>(
    `select coalesce(sum(amount_minor), 0)::bigint as balance
       from ledger_entries where account_id = $1`,
    [accountId],
  );
  return Number(rows[0]!.balance);
}

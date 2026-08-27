import { query } from '../../db/pool';
import { TERMINAL_STATUSES } from '../transfers/state';

/** Transfers that are neither terminal nor completed — the "in progress" count. */
export async function countInProgress(customerId: string): Promise<number> {
  const terminal = [...TERMINAL_STATUSES];
  const { rows } = await query<{ n: string }>(
    `select count(*)::int as n
       from transfers
      where customer_id = $1
        and current_status <> all($2::text[])`,
    [customerId, terminal],
  );
  return Number(rows[0]?.n ?? 0);
}

/**
 * 30-day USD-denominated send volume. Mixed-currency aggregation needs a
 * product decision, so for now only USD-send transfers are summed — a real
 * number rather than the mock's hardcoded one.
 */
export async function usdSendVolume30d(customerId: string): Promise<number> {
  const { rows } = await query<{ total: string }>(
    `select coalesce(sum(send_amount_minor), 0)::bigint as total
       from transfers
      where customer_id = $1
        and send_currency = 'USD'
        and created_at >= now() - interval '30 days'`,
    [customerId],
  );
  return Number(rows[0]?.total ?? 0);
}

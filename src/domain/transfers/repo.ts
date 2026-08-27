import type { PoolClient } from 'pg';
import { query } from '../../db/pool';
import type {
  CurrencyCode,
  FirmQuote,
  Transfer,
  TransferState,
  TransferStateHistoryEntry,
  TransferStatus,
} from '../../contract';
import { isUuid } from '../../util/ids';
import { buildState, isTerminal } from './state';

interface TransferRow {
  id: string;
  reference: string;
  customer_id: string;
  idempotency_key: string;
  recipient_id: string;
  send_currency: CurrencyCode;
  receive_currency: CurrencyCode;
  send_amount_minor: string;
  receive_amount_minor: string;
  trade_description: string | null;
  quote_snapshot: FirmQuote;
  current_status: TransferStatus;
  created_at: Date;
  updated_at: Date;
  state_entered_at: Date | null;
  state_payload: Record<string, unknown> | null;
}

function toTransfer(row: TransferRow): Transfer {
  const enteredAt = (row.state_entered_at ?? row.created_at).toISOString();
  const state: TransferState = buildState(row.current_status, enteredAt, row.state_payload);
  return {
    id: row.id,
    reference: row.reference,
    customerId: row.customer_id,
    idempotencyKey: row.idempotency_key,
    recipientId: row.recipient_id,
    sendCurrency: row.send_currency,
    receiveCurrency: row.receive_currency,
    sendAmount: { amountMinor: Number(row.send_amount_minor), currency: row.send_currency },
    receiveAmount: {
      amountMinor: Number(row.receive_amount_minor),
      currency: row.receive_currency,
    },
    ...(row.trade_description ? { tradeDescription: row.trade_description } : {}),
    quote: row.quote_snapshot,
    state,
    createdAt: row.created_at.toISOString(),
    updatedAt: row.updated_at.toISOString(),
  };
}

const SELECT = `
  select t.*,
         h.entered_at as state_entered_at,
         h.payload    as state_payload
    from transfers t
    left join lateral (
      select entered_at, payload
        from transfer_state_history
       where transfer_id = t.id
       order by position desc
       limit 1
    ) h on true
`;

export async function findById(id: string): Promise<Transfer | null> {
  if (!isUuid(id)) return null;
  const { rows } = await query<TransferRow>(`${SELECT} where t.id = $1`, [id]);
  return rows[0] ? toTransfer(rows[0]) : null;
}

export async function findByIdempotencyKey(
  customerId: string,
  key: string,
): Promise<Transfer | null> {
  const { rows } = await query<TransferRow>(
    `${SELECT} where t.customer_id = $1 and t.idempotency_key = $2`,
    [customerId, key],
  );
  return rows[0] ? toTransfer(rows[0]) : null;
}

export async function listByCustomer(
  customerId: string,
  status?: TransferStatus,
): Promise<Transfer[]> {
  const params: unknown[] = [customerId];
  let where = `where t.customer_id = $1`;
  if (status) {
    params.push(status);
    where += ` and t.current_status = $2`;
  }
  const { rows } = await query<TransferRow>(
    `${SELECT} ${where} order by t.created_at desc`,
    params,
  );
  return rows.map(toTransfer);
}

export async function getOwnerAndStatus(
  id: string,
): Promise<{ customerId: string; currentStatus: TransferStatus } | null> {
  if (!isUuid(id)) return null;
  const { rows } = await query<{ customer_id: string; current_status: TransferStatus }>(
    `select customer_id, current_status from transfers where id = $1`,
    [id],
  );
  const row = rows[0];
  return row ? { customerId: row.customer_id, currentStatus: row.current_status } : null;
}

export async function getHistory(transferId: string): Promise<TransferStateHistoryEntry[]> {
  const { rows } = await query<{ status: TransferStatus; entered_at: Date; note: string | null }>(
    `select status, entered_at, note
       from transfer_state_history
      where transfer_id = $1
      order by position`,
    [transferId],
  );
  return rows.map((r) => ({
    status: r.status,
    enteredAt: r.entered_at.toISOString(),
    ...(r.note ? { note: r.note } : {}),
  }));
}

export function timelineIsTerminal(currentStatus: TransferStatus): boolean {
  return isTerminal(currentStatus);
}

// ---- writes ----

export interface InsertTransfer {
  reference: string;
  customerId: string;
  idempotencyKey: string;
  recipientId: string;
  sendCurrency: CurrencyCode;
  receiveCurrency: CurrencyCode;
  sendAmountMinor: number;
  receiveAmountMinor: number;
  tradeDescription?: string | null;
  quoteSnapshot: FirmQuote;
  initialStatus: TransferStatus;
}

export async function insert(client: PoolClient, input: InsertTransfer): Promise<string> {
  const { rows } = await client.query<{ id: string }>(
    `insert into transfers
       (reference, customer_id, idempotency_key, recipient_id, send_currency, receive_currency,
        send_amount_minor, receive_amount_minor, trade_description, quote_snapshot, current_status)
     values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::jsonb,$11)
     returning id`,
    [
      input.reference,
      input.customerId,
      input.idempotencyKey,
      input.recipientId,
      input.sendCurrency,
      input.receiveCurrency,
      input.sendAmountMinor,
      input.receiveAmountMinor,
      input.tradeDescription ?? null,
      JSON.stringify(input.quoteSnapshot),
      input.initialStatus,
    ],
  );
  return rows[0]!.id;
}

export async function appendHistory(
  client: PoolClient,
  transferId: string,
  status: TransferStatus,
  payload?: Record<string, unknown> | null,
  note?: string,
): Promise<void> {
  await client.query(
    `insert into transfer_state_history (transfer_id, position, status, payload, note)
     values (
       $1,
       (select coalesce(max(position), -1) + 1 from transfer_state_history where transfer_id = $1),
       $2, $3::jsonb, $4
     )`,
    [transferId, status, payload ? JSON.stringify(payload) : null, note ?? null],
  );
}

export async function setStatus(
  client: PoolClient,
  transferId: string,
  status: TransferStatus,
): Promise<void> {
  await client.query(
    `update transfers set current_status = $2, updated_at = now() where id = $1`,
    [transferId, status],
  );
}

import type { PoolClient } from 'pg';
import { query } from '../../db/pool';
import type { CurrencyCode, FirmQuote } from '../../contract';
import { isUuid } from '../../util/ids';

interface QuoteRow {
  id: string;
  customer_id: string;
  send_currency: CurrencyCode;
  receive_currency: CurrencyCode;
  rate: string;
  fee_minor: string;
  send_amount_minor: string;
  receive_amount_minor: string;
  issued_at: Date;
  expires_at: Date;
}

export interface StoredQuote {
  id: string;
  customerId: string;
  firmQuote: FirmQuote;
  expiresAt: Date;
}

function toStored(row: QuoteRow): StoredQuote {
  const rate = Number(row.rate);
  const firmQuote: FirmQuote = {
    id: row.id,
    sendCurrency: row.send_currency,
    receiveCurrency: row.receive_currency,
    breakdown: {
      rate,
      fee: { amountMinor: Number(row.fee_minor), currency: row.send_currency },
      sendAmount: { amountMinor: Number(row.send_amount_minor), currency: row.send_currency },
      receiveAmount: {
        amountMinor: Number(row.receive_amount_minor),
        currency: row.receive_currency,
      },
    },
    issuedAt: row.issued_at.toISOString(),
    expiresAt: row.expires_at.toISOString(),
  };
  return { id: row.id, customerId: row.customer_id, firmQuote, expiresAt: row.expires_at };
}

const COLUMNS = `id, customer_id, send_currency, receive_currency, rate, fee_minor,
                 send_amount_minor, receive_amount_minor, issued_at, expires_at`;

export interface InsertQuote {
  customerId: string;
  sendCurrency: CurrencyCode;
  receiveCurrency: CurrencyCode;
  rate: number;
  feeMinor: number;
  sendAmountMinor: number;
  receiveAmountMinor: number;
  ttlSeconds: number;
}

export async function insert(input: InsertQuote): Promise<StoredQuote> {
  const { rows } = await query<QuoteRow>(
    `insert into quotes
       (customer_id, send_currency, receive_currency, rate, fee_minor,
        send_amount_minor, receive_amount_minor, expires_at)
     values ($1, $2, $3, $4, $5, $6, $7, now() + ($8 || ' seconds')::interval)
     returning ${COLUMNS}`,
    [
      input.customerId,
      input.sendCurrency,
      input.receiveCurrency,
      input.rate,
      input.feeMinor,
      input.sendAmountMinor,
      input.receiveAmountMinor,
      String(input.ttlSeconds),
    ],
  );
  return toStored(rows[0]!);
}

export async function findById(
  id: string,
  client?: PoolClient,
): Promise<StoredQuote | null> {
  if (!isUuid(id)) return null;
  const run = client
    ? client.query.bind(client)
    : (text: string, params: unknown[]) => query<QuoteRow>(text, params);
  const { rows } = (await run(`select ${COLUMNS} from quotes where id = $1`, [id])) as {
    rows: QuoteRow[];
  };
  return rows[0] ? toStored(rows[0]) : null;
}

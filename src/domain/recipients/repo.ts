import type { PoolClient } from 'pg';
import { query } from '../../db/pool';
import type { CurrencyCode, Recipient } from '../../contract';

interface RecipientRow {
  id: string;
  customer_id: string;
  account_name: string;
  account_number: string;
  bank_code: string;
  bank_name: string;
  currency: CurrencyCode;
  country: string;
  validation_status: Recipient['validationStatus'];
  saved_at: Date;
}

function toRecipient(row: RecipientRow): Recipient {
  return {
    id: row.id,
    customerId: row.customer_id,
    accountName: row.account_name,
    accountNumber: row.account_number,
    bankCode: row.bank_code,
    bankName: row.bank_name,
    currency: row.currency,
    country: row.country,
    validationStatus: row.validation_status,
    savedAt: row.saved_at.toISOString(),
  };
}

const COLUMNS = `id, customer_id, account_name, account_number, bank_code,
                 bank_name, currency, country, validation_status, saved_at`;

export async function listByCustomer(customerId: string): Promise<Recipient[]> {
  const { rows } = await query<RecipientRow>(
    `select ${COLUMNS} from recipients where customer_id = $1 order by saved_at desc`,
    [customerId],
  );
  return rows.map(toRecipient);
}

export async function findById(id: string, customerId: string): Promise<Recipient | null> {
  const { rows } = await query<RecipientRow>(
    `select ${COLUMNS} from recipients where id = $1 and customer_id = $2`,
    [id, customerId],
  );
  return rows[0] ? toRecipient(rows[0]) : null;
}

export interface InsertRecipient {
  customerId: string;
  accountName: string;
  accountNumber: string;
  bankCode: string;
  bankName: string;
  currency: CurrencyCode;
  country: string;
}

export async function insert(
  client: PoolClient,
  input: InsertRecipient,
): Promise<Recipient> {
  const { rows } = await client.query<RecipientRow>(
    `insert into recipients
       (customer_id, account_name, account_number, bank_code, bank_name, currency, country, validation_status)
     values ($1, $2, $3, $4, $5, $6, $7, 'valid')
     returning ${COLUMNS}`,
    [
      input.customerId,
      input.accountName,
      input.accountNumber,
      input.bankCode,
      input.bankName,
      input.currency,
      input.country,
    ],
  );
  return toRecipient(rows[0]!);
}

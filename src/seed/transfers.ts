import type { PoolClient } from 'pg';
import type { CurrencyCode, FirmQuote, TransferStatus } from '../contract';
import {
  DEMO_CUSTOMER_ID,
  DEMO_RECIPIENT_AMSTERDAM,
  DEMO_RECIPIENT_GUPTA,
  DEMO_RECIPIENT_KERALA,
  DEMO_RECIPIENT_NATURALIA,
  DEMO_RECIPIENT_ROTTERDAM,
} from './ids';

// Mirrors the five seeded transfers in Kimana_frontend/src/api/mock/seed.ts so
// the dashboard table renders the same rows.

interface SeedTransfer {
  reference: string;
  recipientId: string;
  sendCurrency: CurrencyCode;
  receiveCurrency: CurrencyCode;
  rate: number;
  sendAmountMinor: number;
  receiveAmountMinor: number;
  tradeDescription: string;
  status: TransferStatus;
  statePayload: Record<string, unknown> | null;
}

const SEED_TRANSFERS: readonly SeedTransfer[] = [
  {
    reference: 'TXN-8844',
    recipientId: DEMO_RECIPIENT_AMSTERDAM,
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    rate: 1645.0,
    sendAmountMinor: 4_500_000,
    receiveAmountMinor: 7_402_500_00,
    tradeDescription: 'Cashew export',
    status: 'COMPLETED',
    statePayload: { payoutReference: 'PO-8844' },
  },
  {
    reference: 'TXN-8843',
    recipientId: DEMO_RECIPIENT_KERALA,
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    rate: 1645.0,
    sendAmountMinor: 1_850_000,
    receiveAmountMinor: 3_043_250_00,
    tradeDescription: 'Sesame export',
    status: 'PAYING_OUT',
    statePayload: null,
  },
  {
    reference: 'TXN-8842',
    recipientId: DEMO_RECIPIENT_NATURALIA,
    sendCurrency: 'EUR',
    receiveCurrency: 'NGN',
    rate: 1802.5,
    sendAmountMinor: 2_200_000,
    receiveAmountMinor: 3_965_500_00,
    tradeDescription: 'Hibiscus export',
    status: 'SCREENED',
    statePayload: { hold: false },
  },
  {
    reference: 'TXN-8841',
    recipientId: DEMO_RECIPIENT_ROTTERDAM,
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    rate: 1645.0,
    sendAmountMinor: 7_800_000,
    receiveAmountMinor: 12_831_000_00,
    tradeDescription: 'Cocoa export',
    status: 'COMPLETED',
    statePayload: { payoutReference: 'PO-8841' },
  },
  {
    reference: 'TXN-8840',
    recipientId: DEMO_RECIPIENT_GUPTA,
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    rate: 1645.0,
    sendAmountMinor: 1_200_000,
    receiveAmountMinor: 1_974_000_00,
    tradeDescription: 'Sesame export',
    status: 'REVERSED',
    statePayload: {
      reason: 'Partner returned funds — beneficiary account closed.',
      reversalLedgerEntryId: '00000000-0000-4000-8000-0000000000ff',
    },
  },
];

function snapshotQuote(t: SeedTransfer): FirmQuote {
  return {
    id: `seed_quote_${t.reference}`,
    sendCurrency: t.sendCurrency,
    receiveCurrency: t.receiveCurrency,
    breakdown: {
      rate: t.rate,
      fee: { amountMinor: 0, currency: t.sendCurrency },
      sendAmount: { amountMinor: t.sendAmountMinor, currency: t.sendCurrency },
      receiveAmount: { amountMinor: t.receiveAmountMinor, currency: t.receiveCurrency },
    },
    issuedAt: '2026-08-20T09:00:00.000Z',
    expiresAt: '2026-08-20T09:01:30.000Z',
  };
}

export async function seedTransfers(c: PoolClient): Promise<void> {
  for (let i = 0; i < SEED_TRANSFERS.length; i++) {
    const t = SEED_TRANSFERS[i]!;
    const { rows } = await c.query<{ id: string }>(
      `insert into transfers
         (reference, customer_id, idempotency_key, recipient_id, send_currency, receive_currency,
          send_amount_minor, receive_amount_minor, trade_description, quote_snapshot, current_status)
       values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::jsonb,$11)
       returning id`,
      [
        t.reference,
        DEMO_CUSTOMER_ID,
        `seed-${t.reference}`,
        t.recipientId,
        t.sendCurrency,
        t.receiveCurrency,
        t.sendAmountMinor,
        t.receiveAmountMinor,
        t.tradeDescription,
        JSON.stringify(snapshotQuote(t)),
        t.status,
      ],
    );
    await c.query(
      `insert into transfer_state_history (transfer_id, position, status, payload)
       values ($1, 0, $2, $3::jsonb)`,
      [rows[0]!.id, t.status, t.statePayload ? JSON.stringify(t.statePayload) : null],
    );
  }
}

import type { FastifyInstance } from 'fastify';
import { afterAll, beforeAll, beforeEach, expect, test } from 'vitest';
import { closeEverything, pool, setupDatabase, startServer } from './helpers';
import { seed } from '../src/seed/seed';
import { DEMO_ACCOUNT_NGN, DEMO_ACCOUNT_USD, DEMO_RECIPIENT_AMSTERDAM } from '../src/seed/ids';
import {
  advanceToCompletion,
  expireTransfer,
  reverseTransfer,
} from '../src/domain/transfers/engine';

let app: FastifyInstance;

beforeAll(async () => {
  await setupDatabase();
  app = await startServer();
});

afterAll(async () => {
  await closeEverything(app);
  await pool.end();
});

beforeEach(async () => {
  await seed();
});

async function createTransfer(sendMinor: number): Promise<{ id: string; receiveMinor: number }> {
  const quote = (
    await app.inject({
      method: 'POST',
      url: '/quotes',
      payload: {
        sendCurrency: 'USD',
        receiveCurrency: 'NGN',
        amount: { amountMinor: sendMinor, currency: 'USD' },
        amountField: 'send',
      },
    })
  ).json();
  const transfer = (
    await app.inject({
      method: 'POST',
      url: '/transfers',
      payload: {
        idempotencyKey: `engine-${sendMinor}-${Math.abs(Math.round(sendMinor / 7))}`,
        quoteId: quote.id,
        recipientId: DEMO_RECIPIENT_AMSTERDAM,
      },
    })
  ).json();
  return { id: transfer.id, receiveMinor: quote.breakdown.receiveAmount.amountMinor };
}

const balance = async (accountId: string) =>
  Number(
    (
      await pool.query<{ b: string }>(
        `select coalesce(sum(amount_minor),0)::bigint as b from ledger_entries where account_id = $1`,
        [accountId],
      )
    ).rows[0].b,
  );

test('happy path drives AWAITING_FUNDS -> COMPLETED with three ledger postings', async () => {
  const { id, receiveMinor } = await createTransfer(5_000_00);

  const final = await advanceToCompletion(id);
  expect(final).toBe('COMPLETED');

  const transfer = (await app.inject({ method: 'GET', url: `/transfers/${id}` })).json();
  expect(transfer.state).toMatchObject({ status: 'COMPLETED' });
  expect(transfer.state.payoutReference).toMatch(/^PO-[A-Z0-9]{6}$/);

  const entries = (
    await pool.query<{ amount_minor: string; description: string; running_balance_minor: string }>(
      `select amount_minor, description, running_balance_minor
         from ledger_entries where transfer_id = $1 order by posted_at, id`,
      [id],
    )
  ).rows;
  expect(entries).toHaveLength(3);
  expect(entries.map((e) => Number(e.amount_minor))).toEqual([-5_000_00, receiveMinor, -receiveMinor]);
  expect(entries[0].description).toContain('funded');
  expect(entries[2].description).toContain('paid to Amsterdam Commodities BV');

  // send account down by the send amount; receive account net zero
  expect(await balance(DEMO_ACCOUNT_USD)).toBe(12_450_000 - 5_000_00);
  expect(await balance(DEMO_ACCOUNT_NGN)).toBe(4_825_000_000);
});

test('running_balance_minor tracks the cumulative account balance', async () => {
  const { id } = await createTransfer(3_000_00);
  await advanceToCompletion(id);

  const rows = (
    await pool.query<{ account_id: string; amount_minor: string; running_balance_minor: string; posted_at: string }>(
      `select account_id, amount_minor, running_balance_minor, posted_at
         from ledger_entries order by account_id, posted_at, id`,
    )
  ).rows;

  const seen = new Map<string, number>();
  for (const r of rows) {
    const next = (seen.get(r.account_id) ?? 0) + Number(r.amount_minor);
    expect(Number(r.running_balance_minor)).toBe(next);
    seen.set(r.account_id, next);
  }
});

test('insufficient send balance rejects at the funding step, no funding entry', async () => {
  // USD balance is 124,500.00; ask for 200,000.00
  const { id } = await createTransfer(200_000_00);
  const final = await advanceToCompletion(id);

  expect(final).toBe('REJECTED');
  const transfer = (await app.inject({ method: 'GET', url: `/transfers/${id}` })).json();
  expect(transfer.state).toMatchObject({
    status: 'REJECTED',
    failureCategory: 'validation',
    reasonCode: 'INSUFFICIENT_FUNDS',
  });

  const count = await pool.query(`select 1 from ledger_entries where transfer_id = $1`, [id]);
  expect(count.rowCount).toBe(0);
  expect(await balance(DEMO_ACCOUNT_USD)).toBe(12_450_000);
});

test('timeline after completion is the full ordered history', async () => {
  const { id } = await createTransfer(1_000_00);
  await advanceToCompletion(id);

  const timeline = (await app.inject({ method: 'GET', url: `/transfers/${id}/timeline` })).json();
  expect(timeline.history.map((h: { status: string }) => h.status)).toEqual([
    'CREATED',
    'QUOTED',
    'SCREENED',
    'AWAITING_FUNDS',
    'FUNDED',
    'SETTLING',
    'SETTLED',
    'PAYING_OUT',
    'COMPLETED',
  ]);
  expect(timeline.isTerminal).toBe(true);
});

test('reverseTransfer from COMPLETED makes the customer whole with a linked entry', async () => {
  const { id } = await createTransfer(4_000_00);
  await advanceToCompletion(id);
  expect(await balance(DEMO_ACCOUNT_USD)).toBe(12_450_000 - 4_000_00);

  const { reversalLedgerEntryId } = await reverseTransfer(id, 'Beneficiary account closed');

  const transfer = (await app.inject({ method: 'GET', url: `/transfers/${id}` })).json();
  expect(transfer.state).toMatchObject({
    status: 'REVERSED',
    reason: 'Beneficiary account closed',
    reversalLedgerEntryId,
  });
  expect(await balance(DEMO_ACCOUNT_USD)).toBe(12_450_000);

  const reversal = (
    await pool.query<{ reversal_of_entry_id: string | null; amount_minor: string }>(
      `select reversal_of_entry_id, amount_minor from ledger_entries where id = $1`,
      [reversalLedgerEntryId],
    )
  ).rows[0];
  expect(Number(reversal.amount_minor)).toBe(4_000_00);
  expect(reversal.reversal_of_entry_id).not.toBeNull();
});

test('an illegal transition is a CONFLICT', async () => {
  const { id } = await createTransfer(1_000_00); // parked at AWAITING_FUNDS
  await expect(reverseTransfer(id, 'too early')).rejects.toMatchObject({ code: 'CONFLICT' });
});

test('expireTransfer from AWAITING_FUNDS is terminal with no ledger effect', async () => {
  const { id } = await createTransfer(1_000_00);
  await expireTransfer(id);

  const transfer = (await app.inject({ method: 'GET', url: `/transfers/${id}` })).json();
  expect(transfer.state.status).toBe('EXPIRED');
  const count = await pool.query(`select 1 from ledger_entries where transfer_id = $1`, [id]);
  expect(count.rowCount).toBe(0);
});

test('a completed transfer is reflected in dashboard balances', async () => {
  const { id } = await createTransfer(10_000_00);
  await advanceToCompletion(id);

  const overview = (await app.inject({ method: 'GET', url: '/dashboard/overview' })).json();
  const usd = overview.balances.find((b: { currency: string }) => b.currency === 'USD');
  expect(usd.balance.amountMinor).toBe(12_450_000 - 10_000_00);
});

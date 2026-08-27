import type { FastifyInstance } from 'fastify';
import { afterAll, beforeAll, beforeEach, expect, test } from 'vitest';
import { closeEverything, pool, setupDatabase, startServer } from './helpers';
import { seed } from '../src/seed/seed';
import { DEMO_RECIPIENT_AMSTERDAM } from '../src/seed/ids';

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

async function freshQuote() {
  const res = await app.inject({
    method: 'POST',
    url: '/quotes',
    payload: {
      sendCurrency: 'USD',
      receiveCurrency: 'NGN',
      amount: { amountMinor: 4_500_000, currency: 'USD' },
      amountField: 'send',
    },
  });
  return res.json();
}

const create = (payload: unknown) =>
  app.inject({ method: 'POST', url: '/transfers', payload });

test('createTransfer snapshots the quote and starts at CREATED', async () => {
  const quote = await freshQuote();
  const res = await create({
    idempotencyKey: 'idem-key-0001',
    quoteId: quote.id,
    recipientId: DEMO_RECIPIENT_AMSTERDAM,
  });
  expect(res.statusCode).toBe(201);
  const t = res.json();
  expect(t.reference).toMatch(/^KM-[A-Z0-9]{6}$/);
  expect(t.state.status).toBe('CREATED');
  expect(t.sendAmount).toEqual({ amountMinor: 4_500_000, currency: 'USD' });
  expect(t.quote.id).toBe(quote.id);
  expect(t.quote.breakdown.rate).toBe(1645.2);
});

test('replaying the same idempotency key returns the same transfer', async () => {
  const quote = await freshQuote();
  const first = (
    await create({ idempotencyKey: 'idem-key-0002', quoteId: quote.id, recipientId: DEMO_RECIPIENT_AMSTERDAM })
  ).json();
  const second = (
    await create({ idempotencyKey: 'idem-key-0002', quoteId: quote.id, recipientId: DEMO_RECIPIENT_AMSTERDAM })
  ).json();

  expect(second.id).toBe(first.id);
  const { rows } = await pool.query(`select count(*)::int as n from transfers where reference = $1`, [
    first.reference,
  ]);
  expect(rows[0].n).toBe(1);
});

test('an expired quote is rejected with RATE_EXPIRED', async () => {
  const quote = await freshQuote();
  await pool.query(`update quotes set expires_at = now() - interval '1 second' where id = $1`, [
    quote.id,
  ]);
  const res = await create({
    idempotencyKey: 'idem-key-0003',
    quoteId: quote.id,
    recipientId: DEMO_RECIPIENT_AMSTERDAM,
  });
  expect(res.statusCode).toBe(409);
  expect(res.json()).toMatchObject({ code: 'RATE_EXPIRED', retryable: true });
});

test('unknown quote or recipient is NOT_FOUND', async () => {
  const quote = await freshQuote();
  expect(
    (await create({ idempotencyKey: 'idem-key-0004', quoteId: 'nope', recipientId: DEMO_RECIPIENT_AMSTERDAM }))
      .statusCode,
  ).toBe(404);
  expect(
    (await create({ idempotencyKey: 'idem-key-0005', quoteId: quote.id, recipientId: 'nope' })).statusCode,
  ).toBe(404);
});

test('createTransfer accepts the idempotency key from a header', async () => {
  const quote = await freshQuote();
  const res = await app.inject({
    method: 'POST',
    url: '/transfers',
    headers: { 'idempotency-key': 'header-key-000001' },
    payload: { quoteId: quote.id, recipientId: DEMO_RECIPIENT_AMSTERDAM },
  });
  expect(res.statusCode).toBe(201);
  expect(res.json().idempotencyKey).toBe('header-key-000001');
});

test('GET /transfers lists seeded transfers and filters by status', async () => {
  const all = await app.inject({ method: 'GET', url: '/transfers' });
  expect(all.json()).toHaveLength(5);

  const completed = await app.inject({ method: 'GET', url: '/transfers?status=COMPLETED' });
  expect(completed.json()).toHaveLength(2);
  expect(completed.json().every((t: { state: { status: string } }) => t.state.status === 'COMPLETED')).toBe(
    true,
  );
});

test('seeded transfer states round-trip their payload', async () => {
  const list = await app.inject({ method: 'GET', url: '/transfers?status=COMPLETED' });
  const completed = list.json()[0];
  expect(completed.state.payoutReference).toMatch(/^PO-/);

  const screened = (await app.inject({ method: 'GET', url: '/transfers?status=SCREENED' })).json()[0];
  expect(screened.state).toMatchObject({ status: 'SCREENED', hold: false });
});

test('GET /transfers/:id/timeline returns history and terminal flag', async () => {
  const reversed = (await app.inject({ method: 'GET', url: '/transfers?status=REVERSED' })).json()[0];
  const timeline = await app.inject({ method: 'GET', url: `/transfers/${reversed.id}/timeline` });
  expect(timeline.statusCode).toBe(200);
  expect(timeline.json().isTerminal).toBe(true);
  expect(timeline.json().history[0].status).toBe('REVERSED');
});

test('another read of an unknown transfer id is NOT_FOUND', async () => {
  const res = await app.inject({
    method: 'GET',
    url: '/transfers/00000000-0000-4000-8000-0000000000aa',
  });
  expect(res.statusCode).toBe(404);
});

test('dashboard in-progress count and USD volume come from the transfers table', async () => {
  const overview = (await app.inject({ method: 'GET', url: '/dashboard/overview' })).json();
  // Seeded: PAYING_OUT + SCREENED are in progress; COMPLETED/REVERSED are not.
  expect(overview.stats.transfersInProgress).toBe(2);
  // USD sends: 45k + 18.5k + 78k + 12k = 153,500.00
  expect(overview.stats.volume30d).toEqual({ amountMinor: 153_500_00, currency: 'USD' });
});

import type { FastifyInstance } from 'fastify';
import { afterAll, beforeAll, beforeEach, expect, test } from 'vitest';
import { closeEverything, pool, setupDatabase, startServer } from './helpers';
import { seed } from '../src/seed/seed';

let app: FastifyInstance;

const quote = (payload: unknown) =>
  app.inject({ method: 'POST', url: '/quotes', payload });

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

test('firm quote from a send amount derives the receive amount at the rate', async () => {
  const res = await quote({
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    amount: { amountMinor: 4_500_000, currency: 'USD' },
    amountField: 'send',
  });
  expect(res.statusCode).toBe(201);
  const body = res.json();
  expect(body.breakdown.rate).toBe(1645.2);
  expect(body.breakdown.sendAmount).toEqual({ amountMinor: 4_500_000, currency: 'USD' });
  expect(body.breakdown.receiveAmount).toEqual({
    amountMinor: Math.round(4_500_000 * 1645.2),
    currency: 'NGN',
  });
  expect(body.breakdown.fee).toEqual({ amountMinor: 0, currency: 'USD' });
});

test('firm quote from a receive amount derives the send amount', async () => {
  const res = await quote({
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    amount: { amountMinor: 1_645_200_00, currency: 'NGN' },
    amountField: 'receive',
  });
  const body = res.json();
  expect(body.breakdown.sendAmount.amountMinor).toBe(Math.round(1_645_200_00 / 1645.2));
  expect(body.breakdown.receiveAmount.amountMinor).toBe(1_645_200_00);
});

test('expiresAt is issuedAt + 90s', async () => {
  const body = (
    await quote({
      sendCurrency: 'USD',
      receiveCurrency: 'NGN',
      amount: { amountMinor: 100_000, currency: 'USD' },
      amountField: 'send',
    })
  ).json();
  const delta = new Date(body.expiresAt).getTime() - new Date(body.issuedAt).getTime();
  expect(delta).toBe(90_000);
});

test('amount currency must match the entered side', async () => {
  const res = await quote({
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    amount: { amountMinor: 100, currency: 'NGN' },
    amountField: 'send',
  });
  expect(res.statusCode).toBe(400);
  expect(res.json().code).toBe('VALIDATION');
});

test('unknown corridor is VALIDATION', async () => {
  const res = await quote({
    sendCurrency: 'GBP',
    receiveCurrency: 'EUR',
    amount: { amountMinor: 100, currency: 'GBP' },
    amountField: 'send',
  });
  expect(res.statusCode).toBe(400);
});

test('same send and receive currency is rejected', async () => {
  const res = await quote({
    sendCurrency: 'USD',
    receiveCurrency: 'USD',
    amount: { amountMinor: 100, currency: 'USD' },
    amountField: 'send',
  });
  expect(res.statusCode).toBe(400);
});

test('quote is persisted for the customer', async () => {
  const body = (
    await quote({
      sendCurrency: 'EUR',
      receiveCurrency: 'NGN',
      amount: { amountMinor: 200_000, currency: 'EUR' },
      amountField: 'send',
    })
  ).json();
  const { rows } = await pool.query(`select id from quotes where id = $1`, [body.id]);
  expect(rows).toHaveLength(1);
});

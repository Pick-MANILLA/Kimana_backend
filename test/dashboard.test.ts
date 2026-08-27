import type { FastifyInstance } from 'fastify';
import { afterAll, beforeAll, beforeEach, expect, test } from 'vitest';
import { closeEverything, pool, setupDatabase, startServer } from './helpers';
import { seed } from '../src/seed/seed';

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

test('GET /session returns the seeded demo customer', async () => {
  const res = await app.inject({ method: 'GET', url: '/session' });
  expect(res.statusCode).toBe(200);
  expect(res.json()).toMatchObject({ role: 'customer', displayName: 'Chinonso' });
});

test('GET /dashboard/overview sums ledger entries into per-currency balances', async () => {
  const res = await app.inject({ method: 'GET', url: '/dashboard/overview' });
  expect(res.statusCode).toBe(200);
  const body = res.json();

  const byCurrency = Object.fromEntries(
    body.balances.map((b: { currency: string; balance: { amountMinor: number } }) => [
      b.currency,
      b.balance.amountMinor,
    ]),
  );
  // Opening balances from seed.ts / the frontend mock store.
  expect(byCurrency).toEqual({ NGN: 4_825_000_000, USD: 12_450_000, EUR: 1_820_000 });

  expect(body.displayName).toBe('Chinonso');
  expect(body.businessName).toBe('Adunola Exports Ltd');
  expect(body.accountId).toBe('AEL-00029');
  expect(body.stats.payoutSuccessRatePercent).toBe(98.3);
  expect(body.pendingActions).toHaveLength(3);
  expect(body.workingCapitalOffer.maxAdvance.currency).toBe('USD');
});

test('overview reflects saved business name and first principal after onboarding', async () => {
  await app.inject({
    method: 'PUT',
    url: '/onboarding/application/business',
    payload: {
      business: {
        legalName: 'Kano Cashew Traders',
        cacNumber: 'RC-9090909',
        businessType: 'partnership',
        industry: 'trading_commodities',
        tradingAddress: { state: 'Kano', country: 'NG' },
        countryOfIncorporation: 'NG',
      },
    },
  });
  await app.inject({
    method: 'PUT',
    url: '/onboarding/application/principals',
    payload: {
      principals: [
        {
          fullName: 'Amina Bello',
          role: 'director',
          dateOfBirth: '1990-01-01',
          bvn: '11111111111',
          nin: '22222222222',
        },
      ],
    },
  });

  const res = await app.inject({ method: 'GET', url: '/dashboard/overview' });
  const body = res.json();
  expect(body.businessName).toBe('Kano Cashew Traders');
  expect(body.displayName).toBe('Amina');
});

test('/health does not require the database session', async () => {
  const res = await app.inject({ method: 'GET', url: '/health' });
  expect(res.json()).toEqual({ ok: true });
});

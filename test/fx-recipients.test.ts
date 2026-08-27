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

test('GET /rates/indicative returns the seeded pair', async () => {
  const res = await app.inject({ method: 'GET', url: '/rates/indicative?send=USD&receive=NGN' });
  expect(res.statusCode).toBe(200);
  expect(res.json()).toMatchObject({
    sendCurrency: 'USD',
    receiveCurrency: 'NGN',
    rate: 1645.2,
    changePercent24h: 0.32,
  });
  expect(typeof res.json().asOf).toBe('string');
});

test('GET /rates/indicative is VALIDATION for an unknown pair', async () => {
  const res = await app.inject({ method: 'GET', url: '/rates/indicative?send=USD&receive=EUR' });
  expect(res.statusCode).toBe(400);
  expect(res.json().code).toBe('VALIDATION');
});

test('GET /rates/indicative rejects a bad currency code', async () => {
  const res = await app.inject({ method: 'GET', url: '/rates/indicative?send=USD&receive=XYZ' });
  expect(res.statusCode).toBe(400);
});

test('GET /recipients returns the five seeded recipients', async () => {
  const res = await app.inject({ method: 'GET', url: '/recipients' });
  expect(res.statusCode).toBe(200);
  const names = res.json().map((r: { accountName: string }) => r.accountName);
  expect(names).toContain('Amsterdam Commodities BV');
  expect(res.json()).toHaveLength(5);
});

test('validate -> save round-trips a new recipient', async () => {
  const input = { accountNumber: '9876543210', bankCode: '058', currency: 'USD', country: 'US' };

  const validate = await app.inject({
    method: 'POST',
    url: '/recipients/validate',
    payload: input,
  });
  expect(validate.statusCode).toBe(200);
  expect(validate.json().accountName).toBe('Verified Beneficiary (3210)');

  const save = await app.inject({
    method: 'POST',
    url: '/recipients',
    payload: { ...input, accountName: validate.json().accountName },
  });
  expect(save.statusCode).toBe(201);
  expect(save.json()).toMatchObject({
    accountName: 'Verified Beneficiary (3210)',
    bankName: 'Partner Bank',
    validationStatus: 'valid',
    currency: 'USD',
  });

  const list = await app.inject({ method: 'GET', url: '/recipients' });
  expect(list.json()).toHaveLength(6);

  const audit = await pool.query(`select 1 from audit_log where action = 'recipient.saved'`);
  expect(audit.rowCount).toBe(1);
});

test('POST /recipients/validate rejects a too-short account number', async () => {
  const res = await app.inject({
    method: 'POST',
    url: '/recipients/validate',
    payload: { accountNumber: '12', bankCode: '058', currency: 'USD', country: 'US' },
  });
  expect(res.statusCode).toBe(400);
  expect(res.json().code).toBe('VALIDATION');
});

import type { FastifyInstance } from 'fastify';
import { afterAll, beforeAll, beforeEach, expect, test } from 'vitest';
import { closeEverything, pool, setupDatabase, startServer } from './helpers';
import { seed } from '../src/seed/seed';

let app: FastifyInstance;

const VALID_BUSINESS = {
  legalName: 'Adunola Exports Ltd',
  cacNumber: 'RC-1234567',
  businessType: 'limited_liability_company',
  industry: 'agriculture_agro_export',
  tradingAddress: { state: 'Lagos', country: 'NG' },
  countryOfIncorporation: 'NG',
};

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

test('GET /onboarding/application returns the seeded draft', async () => {
  const res = await app.inject({ method: 'GET', url: '/onboarding/application' });
  expect(res.statusCode).toBe(200);
  const body = res.json();
  expect(body.status).toBe('draft');
  expect(body.business).toBeNull();
  expect(body.principals).toEqual([]);
  expect(body.documents).toEqual([]);
});

test('PUT business persists and echoes the details', async () => {
  const res = await app.inject({
    method: 'PUT',
    url: '/onboarding/application/business',
    payload: { business: VALID_BUSINESS },
  });
  expect(res.statusCode).toBe(200);
  expect(res.json().business.legalName).toBe('Adunola Exports Ltd');

  const reread = await app.inject({ method: 'GET', url: '/onboarding/application' });
  expect(reread.json().business.cacNumber).toBe('RC-1234567');
});

test('PUT business rejects a malformed RC number with VALIDATION', async () => {
  const res = await app.inject({
    method: 'PUT',
    url: '/onboarding/application/business',
    payload: { business: { ...VALID_BUSINESS, cacNumber: 'not-a-number' } },
  });
  expect(res.statusCode).toBe(400);
  expect(res.json()).toMatchObject({ code: 'VALIDATION', retryable: false });
});

test('PUT principals replaces the list', async () => {
  const res = await app.inject({
    method: 'PUT',
    url: '/onboarding/application/principals',
    payload: {
      principals: [
        {
          fullName: 'Chinonso Okafor',
          role: 'director',
          dateOfBirth: '1985-04-12',
          bvn: '12345678901',
          nin: '10987654321',
        },
      ],
    },
  });
  expect(res.statusCode).toBe(200);
  const principals = res.json().principals;
  expect(principals).toHaveLength(1);
  expect(principals[0]).toMatchObject({ fullName: 'Chinonso Okafor', role: 'director' });
  expect(principals[0].id).toBeTruthy();
});

test('PUT principals rejects a 10-digit BVN', async () => {
  const res = await app.inject({
    method: 'PUT',
    url: '/onboarding/application/principals',
    payload: { principals: [{ fullName: 'Bad Actor', role: 'director', bvn: '123' }] },
  });
  expect(res.statusCode).toBe(400);
  expect(res.json().code).toBe('VALIDATION');
});

test('submit walks draft -> approved and populates approvedSummary', async () => {
  await app.inject({
    method: 'PUT',
    url: '/onboarding/application/business',
    payload: { business: VALID_BUSINESS },
  });

  const res = await app.inject({ method: 'POST', url: '/onboarding/application/submit', payload: {} });
  expect(res.statusCode).toBe(200);
  const body = res.json();
  expect(body.status).toBe('approved');
  expect(body.submittedAt).toBeTruthy();
  expect(body.reviewedAt).toBeTruthy();
  expect(body.approvedSummary.accountId).toMatch(/^[A-Z]{1,3}-\d{5}$/);
  expect(body.approvedSummary.segment).toBe('Agro Exporter');
  expect(body.approvedSummary.monthlyLimit).toEqual({ amountMinor: 100_000_00, currency: 'USD' });
});

test('submit without business details fails VALIDATION', async () => {
  const res = await app.inject({ method: 'POST', url: '/onboarding/application/submit', payload: {} });
  expect(res.statusCode).toBe(400);
  expect(res.json().code).toBe('VALIDATION');
});

test('submit with a wrong applicationId is NOT_FOUND', async () => {
  await app.inject({
    method: 'PUT',
    url: '/onboarding/application/business',
    payload: { business: VALID_BUSINESS },
  });
  const res = await app.inject({
    method: 'POST',
    url: '/onboarding/application/submit',
    payload: { applicationId: 'nope' },
  });
  expect(res.statusCode).toBe(404);
  expect(res.json().code).toBe('NOT_FOUND');
});

test('every onboarding mutation writes an audit row', async () => {
  await app.inject({
    method: 'PUT',
    url: '/onboarding/application/business',
    payload: { business: VALID_BUSINESS },
  });
  await app.inject({ method: 'POST', url: '/onboarding/application/submit', payload: {} });

  const { rows } = await pool.query<{ action: string }>(
    `select action from audit_log order by occurred_at`,
  );
  const actions = rows.map((r) => r.action);
  expect(actions).toContain('onboarding.business_saved');
  // three state transitions on submit: draft->submitted->in_review->approved
  expect(actions.filter((a) => a === 'onboarding.state_change')).toHaveLength(3);
});

test('document upload stores the file and rejects a bad mime type', async () => {
  const boundary = '----kimanatest';
  const pdf = Buffer.from('%PDF-1.4 test');
  const multipart = Buffer.concat([
    Buffer.from(
      `--${boundary}\r\nContent-Disposition: form-data; name="type"\r\n\r\ncac_certificate\r\n` +
        `--${boundary}\r\nContent-Disposition: form-data; name="file"; filename="cac.pdf"\r\n` +
        `Content-Type: application/pdf\r\n\r\n`,
    ),
    pdf,
    Buffer.from(`\r\n--${boundary}--\r\n`),
  ]);

  const ok = await app.inject({
    method: 'POST',
    url: '/onboarding/application/documents',
    headers: { 'content-type': `multipart/form-data; boundary=${boundary}` },
    payload: multipart,
  });
  expect(ok.statusCode).toBe(200);
  expect(ok.json()).toMatchObject({ type: 'cac_certificate', status: 'uploaded' });

  const bad = Buffer.concat([
    Buffer.from(
      `--${boundary}\r\nContent-Disposition: form-data; name="type"\r\n\r\nmemart\r\n` +
        `--${boundary}\r\nContent-Disposition: form-data; name="file"; filename="x.txt"\r\n` +
        `Content-Type: text/plain\r\n\r\nhello\r\n--${boundary}--\r\n`,
    ),
  ]);
  const badRes = await app.inject({
    method: 'POST',
    url: '/onboarding/application/documents',
    headers: { 'content-type': `multipart/form-data; boundary=${boundary}` },
    payload: bad,
  });
  expect(badRes.statusCode).toBe(400);
  expect(badRes.json().code).toBe('VALIDATION');
});

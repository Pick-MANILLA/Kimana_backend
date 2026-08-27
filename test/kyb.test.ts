import type { FastifyInstance } from 'fastify';
import { afterAll, beforeAll, beforeEach, expect, test } from 'vitest';
import { closeEverything, pool, setupDatabase, startServer } from './helpers';
import { seed } from '../src/seed/seed';

let app: FastifyInstance;

const business = (overrides: Record<string, unknown> = {}) => ({
  legalName: 'Adunola Exports Ltd',
  cacNumber: 'RC-1234567',
  businessType: 'limited_liability_company',
  industry: 'agriculture_agro_export',
  tradingAddress: { state: 'Lagos', country: 'NG' },
  countryOfIncorporation: 'NG',
  ...overrides,
});

async function saveBusiness(overrides?: Record<string, unknown>) {
  return app.inject({
    method: 'PUT',
    url: '/onboarding/application/business',
    payload: { business: business(overrides) },
  });
}

async function savePrincipals(principals: unknown[]) {
  return app.inject({
    method: 'PUT',
    url: '/onboarding/application/principals',
    payload: { principals },
  });
}

const submit = () => app.inject({ method: 'POST', url: '/onboarding/application/submit', payload: {} });

const auditActions = async () =>
  (await pool.query<{ action: string; after: unknown }>(`select action, after from audit_log order by occurred_at`)).rows;

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

test('clean data is approved and records five passing checks', async () => {
  await saveBusiness();
  const res = await submit();
  expect(res.statusCode).toBe(200);
  expect(res.json().status).toBe('approved');
  expect(res.json().rejectionReasons).toBeUndefined();

  const { rows } = await pool.query<{ check_key: string; passed: boolean }>(
    `select check_key, passed from kyb_checks`,
  );
  expect(rows).toHaveLength(5);
  expect(rows.every((r) => r.passed)).toBe(true);
});

test('submit passes through in_review before a terminal status', async () => {
  await saveBusiness();
  await submit();
  const actions = (await auditActions()).map((r) => (r.after as { status?: string })?.status);
  expect(actions).toContain('submitted');
  expect(actions).toContain('in_review');
  expect(actions).toContain('approved');
  expect(actions.indexOf('in_review')).toBeLessThan(actions.indexOf('approved'));
});

test('a legal name flagged by the CAC check is rejected with reasons', async () => {
  await saveBusiness({ legalName: 'Reject Me Traders Ltd' });
  const res = await submit();

  expect(res.statusCode).toBe(200);
  const body = res.json();
  expect(body.status).toBe('rejected');
  expect(body.approvedSummary).toBeUndefined();
  expect(body.rejectionReasons).toEqual(
    expect.arrayContaining([expect.objectContaining({ field: 'business.cacNumber' })]),
  );

  const failed = await pool.query<{ check_key: string }>(
    `select check_key from kyb_checks where passed = false`,
  );
  expect(failed.rows.map((r) => r.check_key)).toContain('cac_lookup');

  const actions = (await auditActions()).map((r) => (r.after as { status?: string })?.status);
  expect(actions).toContain('rejected');
});

test('a sanctioned principal is rejected by the sanctions check', async () => {
  await saveBusiness();
  await savePrincipals([
    {
      fullName: 'Sanctioned Person',
      role: 'director',
      dateOfBirth: '1980-02-02',
      bvn: '12345678901',
      nin: '10987654321',
    },
  ]);
  const res = await submit();
  expect(res.json().status).toBe('rejected');
  expect(res.json().rejectionReasons).toEqual(
    expect.arrayContaining([expect.objectContaining({ field: 'principals[].fullName' })]),
  );
});

test('rejected -> fix -> resubmit clears the reasons and approves', async () => {
  await saveBusiness({ legalName: 'Reject Me Traders Ltd' });
  expect((await submit()).json().status).toBe('rejected');

  await saveBusiness({ legalName: 'Clean Traders Ltd' });
  const res = await submit();
  const body = res.json();

  expect(body.status).toBe('approved');
  expect(body.rejectionReasons).toBeUndefined();
  expect(body.approvedSummary.accountId).toMatch(/^[A-Z]{1,3}-\d{5}$/);
});

test('submitting again while already in review is a CONFLICT', async () => {
  await saveBusiness();
  await pool.query(`update onboarding_applications set status = 'in_review'`);
  const res = await submit();
  expect(res.statusCode).toBe(409);
  expect(res.json().code).toBe('CONFLICT');
});

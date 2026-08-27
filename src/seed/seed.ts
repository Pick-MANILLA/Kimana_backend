import { pool, withTx } from '../db/pool';
import {
  DEMO_ACCOUNT_EUR,
  DEMO_ACCOUNT_NGN,
  DEMO_ACCOUNT_USD,
  DEMO_APPLICATION_ID,
  DEMO_CUSTOMER_ID,
  DEMO_USER_ID,
} from './ids';

/**
 * Mirrors Kimana_frontend/src/api/mock/seed.ts `createMockStore()`:
 * one demo customer ("Chinonso" / Adunola Exports Ltd) with a draft
 * onboarding application and three currency accounts holding the same
 * opening balances the mock ships.
 *
 * Dev-only. Truncates the slice tables and rebuilds them.
 */

const OPENING_BALANCES: ReadonlyArray<readonly [id: string, currency: string, minor: number]> = [
  [DEMO_ACCOUNT_NGN, 'NGN', 4_825_000_000],
  [DEMO_ACCOUNT_USD, 'USD', 12_450_000],
  [DEMO_ACCOUNT_EUR, 'EUR', 1_820_000],
];

export async function seed(): Promise<void> {
  await withTx(async (c) => {
    // Dev reset. audit_log is append-only in normal operation; TRUNCATE
    // bypasses the row trigger and is fine for a local reseed.
    await c.query(`
      truncate audit_log, ledger_entries, onboarding_documents, onboarding_principals,
               onboarding_applications, accounts, customers, users
      restart identity cascade
    `);

    await c.query(
      `insert into users (id, role, display_name, operator_permissions)
       values ($1, 'customer', 'Chinonso', '{}')`,
      [DEMO_USER_ID],
    );

    await c.query(
      `insert into customers (id, legal_name, primary_user_id) values ($1, $2, $3)`,
      [DEMO_CUSTOMER_ID, 'Adunola Exports Ltd', DEMO_USER_ID],
    );

    await c.query(
      `insert into onboarding_applications (id, customer_id, status) values ($1, $2, 'draft')`,
      [DEMO_APPLICATION_ID, DEMO_CUSTOMER_ID],
    );

    for (const [id, currency, minor] of OPENING_BALANCES) {
      await c.query(
        `insert into accounts (id, customer_id, currency) values ($1, $2, $3)`,
        [id, DEMO_CUSTOMER_ID, currency],
      );
      await c.query(
        `insert into ledger_entries
           (account_id, amount_minor, currency, running_balance_minor, description)
         values ($1, $2, $3, $2, 'Opening balance')`,
        [id, minor, currency],
      );
    }
  });
}

if (require.main === module) {
  seed()
    .then(() => {
      console.log('seed complete');
      return pool.end();
    })
    .catch((err) => {
      console.error(err);
      process.exit(1);
    });
}

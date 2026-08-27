import { pool, withTx } from '../db/pool';
import {
  DEMO_ACCOUNT_EUR,
  DEMO_ACCOUNT_NGN,
  DEMO_ACCOUNT_USD,
  DEMO_APPLICATION_ID,
  DEMO_CUSTOMER_ID,
  DEMO_RECIPIENT_AMSTERDAM,
  DEMO_RECIPIENT_GUPTA,
  DEMO_RECIPIENT_KERALA,
  DEMO_RECIPIENT_NATURALIA,
  DEMO_RECIPIENT_ROTTERDAM,
  DEMO_USER_ID,
} from './ids';
import { seedTransfers } from './transfers';

/**
 * Mirrors Kimana_frontend/src/api/mock/seed.ts `createMockStore()`:
 * one demo customer ("Chinonso" / Adunola Exports Ltd) with a draft
 * onboarding application, three currency accounts, five saved recipients,
 * and the indicative FX feed.
 *
 * Dev-only. Truncates the app tables and rebuilds them.
 */

const OPENING_BALANCES: ReadonlyArray<readonly [id: string, currency: string, minor: number]> = [
  [DEMO_ACCOUNT_NGN, 'NGN', 4_825_000_000],
  [DEMO_ACCOUNT_USD, 'USD', 12_450_000],
  [DEMO_ACCOUNT_EUR, 'EUR', 1_820_000],
];

const FX_RATES: ReadonlyArray<readonly [pair: string, rate: number, change24h: number]> = [
  ['USD/NGN', 1645.2, 0.32],
  ['EUR/NGN', 1802.5, -0.11],
  ['GBP/NGN', 2088.4, 0.18],
  ['GHS/NGN', 110.25, -0.44],
];

const RECIPIENTS: ReadonlyArray<
  readonly [id: string, accountName: string, country: string, currency: string]
> = [
  [DEMO_RECIPIENT_AMSTERDAM, 'Amsterdam Commodities BV', 'NL', 'USD'],
  [DEMO_RECIPIENT_KERALA, 'Kerala Spices Corp', 'IN', 'USD'],
  [DEMO_RECIPIENT_NATURALIA, 'Naturalia Foods GmbH', 'DE', 'EUR'],
  [DEMO_RECIPIENT_ROTTERDAM, 'Rotterdam Grain Exchange', 'NL', 'USD'],
  [DEMO_RECIPIENT_GUPTA, 'Gupta Trading India Pvt Ltd', 'IN', 'USD'],
];

export async function seed(): Promise<void> {
  await withTx(async (c) => {
    // Dev reset. audit_log / ledger_entries are append-only in normal operation;
    // TRUNCATE bypasses the row triggers and is fine for a local reseed.
    await c.query(`
      truncate audit_log, ledger_entries, transfer_state_history, transfers, quotes,
               recipients, fx_rates, kyb_checks, onboarding_documents,
               onboarding_principals, onboarding_applications, accounts, customers, users
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
      await c.query(`insert into accounts (id, customer_id, currency) values ($1, $2, $3)`, [
        id,
        DEMO_CUSTOMER_ID,
        currency,
      ]);
      await c.query(
        `insert into ledger_entries
           (account_id, amount_minor, currency, running_balance_minor, description)
         values ($1, $2, $3, $2, 'Opening balance')`,
        [id, minor, currency],
      );
    }

    for (const [pair, rate, change24h] of FX_RATES) {
      await c.query(
        `insert into fx_rates (pair, rate, change_percent_24h) values ($1, $2, $3)`,
        [pair, rate, change24h],
      );
    }

    for (const [id, accountName, country, currency] of RECIPIENTS) {
      await c.query(
        `insert into recipients
           (id, customer_id, account_name, account_number, bank_code, bank_name, currency, country, validation_status)
         values ($1, $2, $3, '0000000000', '000', 'Partner Bank', $4, $5, 'valid')`,
        [id, DEMO_CUSTOMER_ID, accountName, currency, country],
      );
    }

    await seedTransfers(c);
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

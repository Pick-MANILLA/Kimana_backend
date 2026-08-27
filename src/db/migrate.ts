import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { pool } from './pool';

const MIGRATIONS_DIR = join(__dirname, '..', '..', 'migrations');

/**
 * Minimal forward-only migration runner: applies every unapplied `*.sql` file
 * in ./migrations in lexical order, each in its own transaction, and records
 * it in schema_migrations. No down migrations by design.
 */
export async function runMigrations(): Promise<string[]> {
  await pool.query(`
    create table if not exists schema_migrations (
      id          text primary key,
      applied_at  timestamptz not null default now()
    )
  `);

  const applied = new Set(
    (await pool.query<{ id: string }>('select id from schema_migrations')).rows.map((r) => r.id),
  );

  const files = readdirSync(MIGRATIONS_DIR)
    .filter((f) => f.endsWith('.sql'))
    .sort();

  const ran: string[] = [];
  for (const file of files) {
    if (applied.has(file)) continue;
    const sql = readFileSync(join(MIGRATIONS_DIR, file), 'utf8');
    const client = await pool.connect();
    try {
      await client.query('BEGIN');
      await client.query(sql);
      await client.query('insert into schema_migrations (id) values ($1)', [file]);
      await client.query('COMMIT');
      ran.push(file);
    } catch (err) {
      await client.query('ROLLBACK');
      throw new Error(`migration ${file} failed: ${(err as Error).message}`, { cause: err });
    } finally {
      client.release();
    }
  }
  return ran;
}

if (require.main === module) {
  runMigrations()
    .then((ran) => {
      console.log(ran.length ? `applied: ${ran.join(', ')}` : 'migrations already up to date');
      return pool.end();
    })
    .catch((err) => {
      console.error(err);
      process.exit(1);
    });
}

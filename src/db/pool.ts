import { Pool } from 'pg';
import type { PoolClient, QueryResult, QueryResultRow } from 'pg';
import { config } from '../config';

export const pool = new Pool({ connectionString: config.databaseUrl });

/** Convenience wrapper preserving row typing at the call site. */
export function query<T extends QueryResultRow = QueryResultRow>(
  text: string,
  params?: readonly unknown[],
): Promise<QueryResult<T>> {
  return pool.query<T>(text, params as unknown[] | undefined);
}

/**
 * Runs `fn` inside a single transaction. Commits on resolve, rolls back on
 * throw. Every state change in a request handler should go through here so its
 * audit_log row is written atomically with the change itself.
 */
export async function withTx<T>(fn: (client: PoolClient) => Promise<T>): Promise<T> {
  const client = await pool.connect();
  try {
    await client.query('BEGIN');
    const result = await fn(client);
    await client.query('COMMIT');
    return result;
  } catch (err) {
    await client.query('ROLLBACK');
    throw err;
  } finally {
    client.release();
  }
}

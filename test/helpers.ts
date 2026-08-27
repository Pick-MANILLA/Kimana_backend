import type { FastifyInstance } from 'fastify';
import { runMigrations } from '../src/db/migrate';
import { pool } from '../src/db/pool';
import { seed } from '../src/seed/seed';
import { buildServer } from '../src/server';

export async function setupDatabase(): Promise<void> {
  await runMigrations();
  await seed();
}

export async function startServer(): Promise<FastifyInstance> {
  const app = await buildServer();
  await app.ready();
  return app;
}

export async function closeEverything(app: FastifyInstance): Promise<void> {
  await app.close();
}

export { pool };

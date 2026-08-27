import { config } from './config';
import { runMigrations } from './db/migrate';
import { pool } from './db/pool';
import { buildServer } from './server';

async function main(): Promise<void> {
  const ran = await runMigrations();
  if (ran.length) console.log(`applied migrations: ${ran.join(', ')}`);

  const app = await buildServer();
  const address = await app.listen({ port: config.port, host: config.host });
  app.log.info(`kimana-backend listening on ${address}`);

  for (const signal of ['SIGINT', 'SIGTERM'] as const) {
    process.on(signal, () => {
      void app
        .close()
        .then(() => pool.end())
        .then(() => process.exit(0));
    });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

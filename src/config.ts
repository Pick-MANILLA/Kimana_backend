import 'dotenv/config';

/** Process configuration, resolved once at import. */
export const config = {
  env: process.env.NODE_ENV ?? 'development',
  port: Number(process.env.PORT ?? 4000),
  host: process.env.HOST ?? '0.0.0.0',
  databaseUrl:
    process.env.DATABASE_URL ?? 'postgres://kimana:kimana@localhost:5432/kimana',
  storageDir: process.env.STORAGE_DIR ?? '.storage',
  corsOrigin: process.env.CORS_ORIGIN ?? 'http://localhost:5173',
  logLevel: process.env.LOG_LEVEL ?? 'info',
} as const;

export const isTest = config.env === 'test' || process.env.VITEST === 'true';

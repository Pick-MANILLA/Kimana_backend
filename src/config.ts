import 'dotenv/config';

const runningTests = process.env.NODE_ENV === 'test' || process.env.VITEST === 'true';

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

  /**
   * Simulated per-check latency for the stub KYB provider. The frontend's
   * VerificationPage animates ~6s regardless, so keep this well under that.
   * Tests force it to 0.
   */
  kybCheckDelayMs: runningTests ? 0 : Number(process.env.KYB_CHECK_DELAY_MS ?? 600),
} as const;

export const isTest = runningTests;


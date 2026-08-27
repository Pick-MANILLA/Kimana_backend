import { defineConfig } from 'vitest/config';

// Integration tests hit a real Postgres (DATABASE_URL). They share one database
// and reseed per-file, so disable cross-file parallelism.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['test/**/*.test.ts'],
    fileParallelism: false,
    hookTimeout: 30_000,
    testTimeout: 15_000,
  },
});

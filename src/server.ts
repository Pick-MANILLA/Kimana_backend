import Fastify from 'fastify';
import type { FastifyInstance } from 'fastify';
import cors from '@fastify/cors';
import multipart from '@fastify/multipart';
import { config, isTest } from './config';
import { registerErrorHandler } from './http/errorHandler';
import { registerAuth } from './http/auth';
import { sessionRoutes } from './routes/session';
import { onboardingRoutes } from './domain/onboarding/routes';
import { dashboardRoutes } from './domain/dashboard/routes';
import { fxRoutes } from './domain/fx/routes';
import { recipientRoutes } from './domain/recipients/routes';

export async function buildServer(): Promise<FastifyInstance> {
  const app = Fastify({
    logger: isTest ? false : { level: config.logLevel },
  });

  await app.register(cors, { origin: config.corsOrigin, credentials: true });
  await app.register(multipart, { limits: { fileSize: 10 * 1024 * 1024, files: 1 } });

  registerErrorHandler(app);

  // Unauthenticated.
  app.get('/health', async () => ({ ok: true }));

  // Everything else runs behind the session hook, encapsulated so /health
  // never touches the database.
  await app.register(async (api) => {
    registerAuth(api);
    await api.register(sessionRoutes);
    await api.register(onboardingRoutes);
    await api.register(dashboardRoutes);
    await api.register(fxRoutes);
    await api.register(recipientRoutes);
  });

  return app;
}

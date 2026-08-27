import type { FastifyInstance } from 'fastify';
import { getOverview } from './service';

export async function dashboardRoutes(app: FastifyInstance): Promise<void> {
  app.get('/dashboard/overview', async (req) => {
    return getOverview(req.session);
  });
}

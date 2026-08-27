import type { FastifyInstance } from 'fastify';
import type { Session } from '../contract';

export async function sessionRoutes(app: FastifyInstance): Promise<void> {
  app.get('/session', async (req): Promise<Session> => {
    const { userId, role, displayName, operatorPermissions } = req.session;
    return {
      userId,
      role,
      displayName,
      ...(operatorPermissions && operatorPermissions.length > 0
        ? { operatorPermissions }
        : {}),
    };
  });
}

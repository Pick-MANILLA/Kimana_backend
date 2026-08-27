import type { FastifyInstance, FastifyRequest } from 'fastify';
import { query } from '../db/pool';
import { ApiError } from '../errors';
import type { OperatorPermission, Session, UserRole } from '../contract';
import { DEMO_USER_ID } from '../seed/ids';

/** Session plus the resolved customer id (the contract's Session omits it). */
export interface RequestSession extends Session {
  customerId: string;
}

declare module 'fastify' {
  interface FastifyRequest {
    session: RequestSession;
  }
}

interface SessionRow {
  user_id: string;
  role: UserRole;
  display_name: string;
  operator_permissions: OperatorPermission[];
  customer_id: string;
}

/**
 * Resolves `request.session` before every handler. P1 slice: a single seeded
 * demo customer, looked up by DEMO_USER_ID. Real login / session issuance is
 * P1-later; only this function changes when it lands.
 */
export function registerAuth(app: FastifyInstance): void {
  app.decorateRequest('session');

  app.addHook('preHandler', async (req) => {
    const { rows } = await query<SessionRow>(
      `select u.id  as user_id,
              u.role as role,
              u.display_name,
              u.operator_permissions,
              c.id as customer_id
         from users u
         join customers c on c.primary_user_id = u.id
        where u.id = $1`,
      [DEMO_USER_ID],
    );

    const row = rows[0];
    if (!row) {
      throw new ApiError('UNAUTHORIZED', 'Demo session is not seeded. Run `npm run seed`.', false);
    }

    req.session = {
      userId: row.user_id,
      role: row.role,
      displayName: row.display_name,
      operatorPermissions: row.operator_permissions ?? [],
      customerId: row.customer_id,
    };
  });
}

/** Guard for the (not-yet-built) /ops surface. */
export function requireOperator(req: FastifyRequest): void {
  if (req.session.role !== 'operator') {
    throw ApiError.forbidden('Operator access only.');
  }
}

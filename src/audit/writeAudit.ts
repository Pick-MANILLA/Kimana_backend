import type { PoolClient } from 'pg';
import type { UserRole } from '../contract';

export interface AuditInput {
  actorId: string | null;
  actorRole: UserRole | null;
  /** e.g. "onboarding.business_saved", "onboarding.state_change". */
  action: string;
  entityType: string;
  entityId: string;
  before?: unknown;
  after?: unknown;
}

/**
 * Appends one audit_log row. MUST be called with the same transaction client
 * as the change it records, so the two commit or roll back together.
 * audit_log is append-only (DB trigger) — there is no update or delete path.
 */
export async function writeAudit(client: PoolClient, entry: AuditInput): Promise<void> {
  await client.query(
    `insert into audit_log (actor_id, actor_role, action, entity_type, entity_id, before, after)
     values ($1, $2, $3, $4, $5, $6, $7)`,
    [
      entry.actorId,
      entry.actorRole,
      entry.action,
      entry.entityType,
      entry.entityId,
      entry.before === undefined ? null : JSON.stringify(entry.before),
      entry.after === undefined ? null : JSON.stringify(entry.after),
    ],
  );
}

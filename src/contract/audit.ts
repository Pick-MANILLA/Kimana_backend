import type { Id, ISODateTime, Paginated } from './common';
import type { UserRole } from './auth';

export interface AuditLogEntry {
  readonly id: Id;
  readonly actorId: Id;
  readonly actorRole: UserRole;
  /** e.g. "transfer.state_change", "screening.decision", "manual_action.approve". */
  readonly action: string;
  readonly entityType: string;
  readonly entityId: Id;
  readonly before?: unknown;
  readonly after?: unknown;
  readonly occurredAt: ISODateTime;
}

export interface AuditLogFilter {
  readonly actorId?: Id;
  readonly entityType?: string;
  readonly entityId?: Id;
  readonly dateFrom?: ISODateTime;
  readonly dateTo?: ISODateTime;
}

/** Read-only. No delete endpoint exists anywhere in this contract, by design. */
export interface AuditApi {
  list(filter?: AuditLogFilter): Promise<Paginated<AuditLogEntry>>;
}

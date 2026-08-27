import type { Id } from './common';

export type UserRole = 'customer' | 'operator';

/**
 * Maker-checker (P0 #10) needs both operator capabilities representable at
 * once: an operator with only 'initiate_action' can never approve their own
 * action, one with only 'approve_action' can never initiate. A stub session
 * with both is how we exercise the "same person can't do both" rule.
 */
export type OperatorPermission = 'initiate_action' | 'approve_action';

export interface Session {
  readonly userId: Id;
  readonly role: UserRole;
  readonly displayName: string;
  readonly operatorPermissions?: readonly OperatorPermission[];
}

export interface AuthApi {
  getSession(): Promise<Session>;
}

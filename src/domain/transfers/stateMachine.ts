import { ApiError } from '../../errors';
import type { TransferStatus } from '../../contract';

/**
 * Allowed transitions. A transition not listed here is a bug or a race — the
 * engine and any ops action must go through assertTransition().
 *
 *   CREATED → QUOTED → SCREENED → AWAITING_FUNDS → FUNDED → SETTLING
 *           → SETTLED → PAYING_OUT → COMPLETED
 *
 * plus EXPIRED (pre-funding timeout), REJECTED (screening / insufficient
 * funds / partner failure), and REVERSING → REVERSED (post-settlement unwind).
 */
export const ALLOWED_TRANSITIONS: Record<TransferStatus, readonly TransferStatus[]> = {
  CREATED: ['QUOTED', 'EXPIRED'],
  QUOTED: ['SCREENED', 'EXPIRED'],
  SCREENED: ['AWAITING_FUNDS', 'REJECTED', 'EXPIRED'],
  AWAITING_FUNDS: ['FUNDED', 'REJECTED', 'EXPIRED'],
  FUNDED: ['SETTLING', 'REJECTED'],
  SETTLING: ['SETTLED', 'REJECTED'],
  SETTLED: ['PAYING_OUT'],
  PAYING_OUT: ['COMPLETED'],
  COMPLETED: ['REVERSING'],
  REJECTED: [],
  EXPIRED: [],
  REVERSING: ['REVERSED'],
  REVERSED: [],
};

/** The single forward step on the happy path, or null at a waypoint/terminal. */
export const FORWARD_STEP: Partial<Record<TransferStatus, TransferStatus>> = {
  CREATED: 'QUOTED',
  QUOTED: 'SCREENED',
  SCREENED: 'AWAITING_FUNDS',
  AWAITING_FUNDS: 'FUNDED',
  FUNDED: 'SETTLING',
  SETTLING: 'SETTLED',
  SETTLED: 'PAYING_OUT',
  PAYING_OUT: 'COMPLETED',
};

export function canTransition(from: TransferStatus, to: TransferStatus): boolean {
  return ALLOWED_TRANSITIONS[from].includes(to);
}

export function assertTransition(from: TransferStatus, to: TransferStatus): void {
  if (!canTransition(from, to)) {
    throw new ApiError('CONFLICT', `Transfer cannot move from ${from} to ${to}.`, false);
  }
}

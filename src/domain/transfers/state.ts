import type { TransferFailureCategory, TransferState, TransferStatus } from '../../contract';

export const TERMINAL_STATUSES: ReadonlySet<TransferStatus> = new Set([
  'COMPLETED',
  'REJECTED',
  'EXPIRED',
  'REVERSED',
]);

export function isTerminal(status: TransferStatus): boolean {
  return TERMINAL_STATUSES.has(status);
}

/** Reconstructs the discriminated TransferState from a status + its stored payload. */
export function buildState(
  status: TransferStatus,
  enteredAt: string,
  payload: Record<string, unknown> | null,
): TransferState {
  const p = payload ?? {};
  const base = { enteredAt };

  switch (status) {
    case 'SCREENED':
      return {
        status,
        ...base,
        hold: Boolean(p.hold),
        ...(p.expectedResolutionBy
          ? { expectedResolutionBy: String(p.expectedResolutionBy) }
          : {}),
      };
    case 'AWAITING_FUNDS':
      return { status, ...base, fundingReference: String(p.fundingReference ?? '') };
    case 'COMPLETED':
      return { status, ...base, payoutReference: String(p.payoutReference ?? '') };
    case 'REJECTED':
      return {
        status,
        ...base,
        failureCategory: (p.failureCategory as TransferFailureCategory) ?? 'partner_failure',
        reasonCode: String(p.reasonCode ?? 'UNKNOWN'),
      };
    case 'REVERSING':
      return { status, ...base, reason: String(p.reason ?? '') };
    case 'REVERSED':
      return {
        status,
        ...base,
        reason: String(p.reason ?? ''),
        reversalLedgerEntryId: String(p.reversalLedgerEntryId ?? ''),
      };
    default:
      return { status, ...base } as TransferState;
  }
}

import type { Id, ISODateTime } from './common';

export type ScreeningOutcome = 'clear' | 'held' | 'rejected';

/**
 * The only screening shape the customer app is allowed to see. No rule
 * names, no scores, no hit detail — ever.
 */
export interface CustomerScreeningStatus {
  readonly transferId: Id;
  readonly outcome: ScreeningOutcome;
  /** Present only when outcome === 'held'. */
  readonly expectedResolutionBy?: ISODateTime;
}

export type RiskRating = 'low' | 'medium' | 'high';

export interface ScreeningHit {
  readonly id: Id;
  readonly ruleName: string;
  readonly matchedList: string;
  readonly score: number;
  readonly detail: string;
}

export type ScreeningQueueStatus = 'pending' | 'decided';
export type ScreeningQueueDecision = 'approve' | 'reject' | 'escalate';

export interface ScreeningQueueItem {
  readonly id: Id;
  readonly transferId: Id;
  readonly customerId: Id;
  readonly riskRating: RiskRating;
  readonly hits: readonly ScreeningHit[];
  readonly status: ScreeningQueueStatus;
  readonly queuedAt: ISODateTime;
  readonly decision?: ScreeningQueueDecision;
  readonly decisionNote?: string;
  readonly decidedBy?: Id;
  readonly decidedAt?: ISODateTime;
}

export interface ScreeningDecisionInput {
  readonly decision: ScreeningQueueDecision;
  /** Required and non-empty — every decision requires a note. */
  readonly note: string;
}

export interface ScreeningApi {
  getCustomerStatus(transferId: Id): Promise<CustomerScreeningStatus>;
  getQueue(filter?: { status?: ScreeningQueueStatus; riskRating?: RiskRating }): Promise<readonly ScreeningQueueItem[]>;
  getQueueItem(id: Id): Promise<ScreeningQueueItem>;
  decide(id: Id, input: ScreeningDecisionInput): Promise<ScreeningQueueItem>;
}

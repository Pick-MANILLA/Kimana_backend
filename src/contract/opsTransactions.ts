import type { CurrencyCode, Id, ISODateTime, Paginated } from './common';
import type { TradeDocument } from './documents';
import type { LedgerEntry } from './ledger';
import type { ScreeningQueueItem } from './screening';
import type { Transfer, TransferStatus, TransferTimeline } from './transfer';

export interface TransactionSearchFilter {
  readonly reference?: string;
  readonly customerId?: Id;
  readonly status?: TransferStatus;
  readonly dateFrom?: ISODateTime;
  readonly dateTo?: ISODateTime;
  readonly amountMinMinor?: number;
  readonly amountMaxMinor?: number;
  readonly currency?: CurrencyCode;
}

export type ManualActionType = 'retry_payout' | 'trigger_reversal' | 'add_note';

/**
 * Every manual action needs a second operator: one initiates, a different
 * one approves. The mock/live layer must reject approveAction when
 * decidedBy would equal initiatedBy.
 */
export type MakerCheckerStatus = 'pending_approval' | 'approved' | 'rejected';

export interface ManualAction {
  readonly id: Id;
  readonly transferId: Id;
  readonly type: ManualActionType;
  readonly note: string;
  readonly initiatedBy: Id;
  readonly initiatedAt: ISODateTime;
  readonly status: MakerCheckerStatus;
  readonly decidedBy?: Id;
  readonly decidedAt?: ISODateTime;
  readonly decisionNote?: string;
}

export interface OpsTransactionDetail {
  readonly transfer: Transfer;
  readonly timeline: TransferTimeline;
  readonly ledgerEntries: readonly LedgerEntry[];
  readonly documents: readonly TradeDocument[];
  readonly screening?: ScreeningQueueItem;
  readonly manualActions: readonly ManualAction[];
}

export interface InitiateManualActionInput {
  readonly type: ManualActionType;
  readonly note: string;
}

export interface OpsTransactionApi {
  search(filter: TransactionSearchFilter): Promise<Paginated<Transfer>>;
  getDetail(transferId: Id): Promise<OpsTransactionDetail>;
  initiateAction(transferId: Id, input: InitiateManualActionInput): Promise<ManualAction>;
  approveAction(actionId: Id, note: string): Promise<ManualAction>;
  rejectAction(actionId: Id, note: string): Promise<ManualAction>;
}

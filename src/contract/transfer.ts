import type { CurrencyCode, Id, ISODateTime, Money } from './common';
import type { FirmQuote } from './quote';

export type BankAccountValidationStatus = 'unvalidated' | 'validating' | 'valid' | 'invalid';

export interface Recipient {
  readonly id: Id;
  readonly customerId: Id;
  readonly accountName: string;
  readonly accountNumber: string;
  readonly bankCode: string;
  readonly bankName: string;
  readonly currency: CurrencyCode;
  /** ISO 3166-1 alpha-2 */
  readonly country: string;
  readonly validationStatus: BankAccountValidationStatus;
  readonly savedAt: ISODateTime;
}

export interface NewRecipientInput {
  readonly accountNumber: string;
  readonly bankCode: string;
  readonly currency: CurrencyCode;
  readonly country: string;
}

export interface RecipientApi {
  listRecipients(customerId: Id): Promise<readonly Recipient[]>;
  /** Resolves the account holder name so the customer can confirm before saving. */
  validateBankAccount(input: NewRecipientInput): Promise<{ accountName: string }>;
  saveRecipient(input: NewRecipientInput & { accountName: string }): Promise<Recipient>;
}

// ---- Transaction state machine (P0 #5) ----
// Each variant is keyed by a literal `status` so a `switch` over it without
// a `default` fails to compile unless every state is handled.

export type TransferStatus =
  | 'CREATED'
  | 'QUOTED'
  | 'SCREENED'
  | 'AWAITING_FUNDS'
  | 'FUNDED'
  | 'SETTLING'
  | 'SETTLED'
  | 'PAYING_OUT'
  | 'COMPLETED'
  | 'REJECTED'
  | 'EXPIRED'
  | 'REVERSING'
  | 'REVERSED';

/** Why a transfer failed. Drives distinct customer copy per category (P0 #7). */
export type TransferFailureCategory = 'network' | 'validation' | 'compliance_hold' | 'partner_failure';

interface TransferStateBase {
  readonly enteredAt: ISODateTime;
}

export interface CreatedState extends TransferStateBase {
  readonly status: 'CREATED';
}
export interface QuotedState extends TransferStateBase {
  readonly status: 'QUOTED';
}
export interface ScreenedState extends TransferStateBase {
  readonly status: 'SCREENED';
  readonly hold: boolean;
  /** Present only when hold === true; shown to the customer as-is, no reasons. */
  readonly expectedResolutionBy?: ISODateTime;
}
export interface AwaitingFundsState extends TransferStateBase {
  readonly status: 'AWAITING_FUNDS';
  readonly fundingReference: string;
}
export interface FundedState extends TransferStateBase {
  readonly status: 'FUNDED';
}
export interface SettlingState extends TransferStateBase {
  readonly status: 'SETTLING';
}
export interface SettledState extends TransferStateBase {
  readonly status: 'SETTLED';
}
export interface PayingOutState extends TransferStateBase {
  readonly status: 'PAYING_OUT';
}
export interface CompletedState extends TransferStateBase {
  readonly status: 'COMPLETED';
  readonly payoutReference: string;
}
export interface RejectedState extends TransferStateBase {
  readonly status: 'REJECTED';
  readonly failureCategory: TransferFailureCategory;
  /** Ops-facing machine code, e.g. "PARTNER_TIMEOUT". Never shown verbatim to the customer. */
  readonly reasonCode: string;
}
export interface ExpiredState extends TransferStateBase {
  readonly status: 'EXPIRED';
}
export interface ReversingState extends TransferStateBase {
  readonly status: 'REVERSING';
  readonly reason: string;
}
export interface ReversedState extends TransferStateBase {
  readonly status: 'REVERSED';
  readonly reason: string;
  readonly reversalLedgerEntryId: Id;
}

export type TransferState =
  | CreatedState
  | QuotedState
  | ScreenedState
  | AwaitingFundsState
  | FundedState
  | SettlingState
  | SettledState
  | PayingOutState
  | CompletedState
  | RejectedState
  | ExpiredState
  | ReversingState
  | ReversedState;

export interface Transfer {
  readonly id: Id;
  /** Human-facing reference shown in UI, e.g. "KM-2H4F9K". */
  readonly reference: string;
  readonly customerId: Id;
  readonly idempotencyKey: string;
  readonly recipientId: Id;
  readonly sendCurrency: CurrencyCode;
  readonly receiveCurrency: CurrencyCode;
  readonly sendAmount: Money;
  readonly receiveAmount: Money;
  /** Short freeform trade purpose, e.g. "Cashew export" — shown as a table subtitle. */
  readonly tradeDescription?: string;
  /** Immutable snapshot of the quote accepted at creation time. */
  readonly quote: FirmQuote;
  readonly state: TransferState;
  readonly createdAt: ISODateTime;
  readonly updatedAt: ISODateTime;
}

export interface TransferStateHistoryEntry {
  readonly status: TransferStatus;
  readonly enteredAt: ISODateTime;
  /** Ops-only annotation, never surfaced to the customer. */
  readonly note?: string;
}

export interface TransferTimeline {
  readonly transferId: Id;
  readonly history: readonly TransferStateHistoryEntry[];
  readonly isTerminal: boolean;
}

export interface CreateTransferInput {
  /** Client-generated, held for the lifetime of the attempt, reused verbatim on retry — never regenerated. */
  readonly idempotencyKey: string;
  readonly quoteId: Id;
  readonly recipientId: Id;
}

export interface TransferApi {
  createTransfer(input: CreateTransferInput): Promise<Transfer>;
  getTransfer(id: Id): Promise<Transfer>;
  getTimeline(id: Id): Promise<TransferTimeline>;
  listTransfers(customerId: Id, filter?: { status?: TransferStatus }): Promise<readonly Transfer[]>;
}

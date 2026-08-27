import type { CurrencyCode, Id, ISODateTime, Money, Paginated } from './common';

export interface LedgerEntry {
  readonly id: Id;
  readonly accountId: Id;
  readonly transferId?: Id;
  /** Signed: positive = credit, negative = debit. */
  readonly amount: Money;
  readonly runningBalance: Money;
  readonly description: string;
  readonly postedAt: ISODateTime;
  /** Present when this entry reverses a prior entry (corrections are new entries, never edits). */
  readonly reversalOfEntryId?: Id;
}

export interface AccountBalance {
  readonly accountId: Id;
  readonly currency: CurrencyCode;
  readonly balance: Money;
  /** Funds inbound but not yet settled into `balance`. */
  readonly pending?: Money;
  readonly asOf: ISODateTime;
}

export interface StatementFilter {
  readonly currency?: CurrencyCode;
  readonly from?: ISODateTime;
  readonly to?: ISODateTime;
}

export interface LedgerApi {
  getBalances(customerId: Id): Promise<readonly AccountBalance[]>;
  getStatement(customerId: Id, filter?: StatementFilter): Promise<Paginated<LedgerEntry>>;
  /** Placeholder — no real export pipeline in the MVP, just a queued request id. */
  requestStatementExport(customerId: Id, filter?: StatementFilter): Promise<{ requestId: Id }>;
}

export interface OpsLedgerApi {
  getAccountEntries(accountId: Id, filter?: StatementFilter): Promise<Paginated<LedgerEntry>>;
}

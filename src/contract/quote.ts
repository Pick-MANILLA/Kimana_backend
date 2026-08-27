import type { CurrencyCode, Id, ISODateTime, Money } from './common';

export interface IndicativeRate {
  readonly sendCurrency: CurrencyCode;
  readonly receiveCurrency: CurrencyCode;
  /** Display-only reference rate. Never used for client-side arithmetic. */
  readonly rate: number;
  /** Percent change over the last 24h, signed. */
  readonly changePercent24h: number;
  readonly asOf: ISODateTime;
}

export interface CostBreakdown {
  readonly rate: number;
  readonly fee: Money;
  readonly sendAmount: Money;
  readonly receiveAmount: Money;
}

export interface FirmQuote {
  readonly id: Id;
  readonly sendCurrency: CurrencyCode;
  readonly receiveCurrency: CurrencyCode;
  readonly breakdown: CostBreakdown;
  readonly issuedAt: ISODateTime;
  /** issuedAt + 90s. Expired quotes are rejected client-side before submit. */
  readonly expiresAt: ISODateTime;
}

export type QuoteAmountField = 'send' | 'receive';

export interface RequestFirmQuoteInput {
  readonly sendCurrency: CurrencyCode;
  readonly receiveCurrency: CurrencyCode;
  readonly amount: Money;
  /** Which side `amount` was entered in; the other side is derived server-side. */
  readonly amountField: QuoteAmountField;
}

export interface QuoteApi {
  getIndicativeRate(sendCurrency: CurrencyCode, receiveCurrency: CurrencyCode): Promise<IndicativeRate>;
  requestFirmQuote(input: RequestFirmQuoteInput): Promise<FirmQuote>;
}

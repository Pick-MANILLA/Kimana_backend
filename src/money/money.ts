import type { CurrencyCode, Money } from '../contract';

/**
 * Server-side money helpers. Mirrors the invariant from
 * Kimana_frontend/src/money/money.ts: a Money value is an integer count of
 * minor units plus a currency code — never a float, never a bare number.
 * Display formatting stays entirely on the client.
 */

export function money(amountMinor: number, currency: CurrencyCode): Money {
  if (!Number.isInteger(amountMinor)) {
    throw new Error(`money() requires an integer minor-unit amount, got ${amountMinor}`);
  }
  return { amountMinor, currency };
}

export function zero(currency: CurrencyCode): Money {
  return { amountMinor: 0, currency };
}

function assertSameCurrency(a: Money, b: Money): void {
  if (a.currency !== b.currency) {
    throw new Error(`Cannot combine ${a.currency} and ${b.currency} amounts`);
  }
}

export function addMoney(a: Money, b: Money): Money {
  assertSameCurrency(a, b);
  return { amountMinor: a.amountMinor + b.amountMinor, currency: a.currency };
}

export function subtractMoney(a: Money, b: Money): Money {
  assertSameCurrency(a, b);
  return { amountMinor: a.amountMinor - b.amountMinor, currency: a.currency };
}

import { ApiError } from '../../errors';
import type { CurrencyCode, IndicativeRate } from '../../contract';
import { stubFxProvider } from './stubProvider';
import type { FxProvider } from './provider';

/** The active FX source. Swap for a real feed. */
export const fxProvider: FxProvider = stubFxProvider;

export function pairKey(send: CurrencyCode, receive: CurrencyCode): string {
  return `${send}/${receive}`;
}

export async function getIndicativeRate(
  sendCurrency: CurrencyCode,
  receiveCurrency: CurrencyCode,
): Promise<IndicativeRate> {
  const pair = pairKey(sendCurrency, receiveCurrency);
  const rate = await fxProvider.currentRate(pair);
  if (!rate) {
    throw ApiError.validation(`No rate available for ${pair}.`);
  }
  return {
    sendCurrency,
    receiveCurrency,
    rate: rate.rate,
    changePercent24h: rate.changePercent24h,
    asOf: rate.asOf,
  };
}

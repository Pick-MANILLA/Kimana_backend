import { isTest } from '../../config';
import type { FxProvider, FxRate } from './provider';
import { getRate, updateRate } from './repo';

// Matches Kimana_frontend/src/api/mock/quoteApi.ts: a small random walk each
// read so the "live" rate visibly moves. Disabled under test for determinism.
const JITTER_SPREAD = 0.004;

export const stubFxProvider: FxProvider = {
  async currentRate(pair): Promise<FxRate | null> {
    const stored = await getRate(pair);
    if (!stored) return null;

    if (isTest) {
      return {
        rate: stored.rate,
        changePercent24h: stored.changePercent24h,
        asOf: stored.asOf.toISOString(),
      };
    }

    const jitter = 1 + (Math.random() - 0.5) * JITTER_SPREAD;
    const drifted = Math.round(stored.rate * jitter * 100) / 100;
    const asOf = await updateRate(pair, drifted);
    return { rate: drifted, changePercent24h: stored.changePercent24h, asOf: asOf.toISOString() };
  },
};

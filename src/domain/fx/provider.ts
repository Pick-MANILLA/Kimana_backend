export interface FxRate {
  /** Reference mid-rate, e.g. 1645.20 for USD/NGN. */
  readonly rate: number;
  /** Signed percent change over the last 24h. */
  readonly changePercent24h: number;
  readonly asOf: string;
}

/**
 * Source of indicative FX rates. The stub reads a seeded cache and jitters it
 * on every read (like the frontend mock); a real feed replaces this with an
 * upstream call + its own refresh cadence. Returns null for an unknown pair.
 */
export interface FxProvider {
  currentRate(pair: string): Promise<FxRate | null>;
}

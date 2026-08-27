import { query } from '../../db/pool';

interface FxRow {
  pair: string;
  rate: string;
  change_percent_24h: string;
  as_of: Date;
}

export interface StoredFxRate {
  pair: string;
  rate: number;
  changePercent24h: number;
  asOf: Date;
}

export async function getRate(pair: string): Promise<StoredFxRate | null> {
  const { rows } = await query<FxRow>(
    `select pair, rate, change_percent_24h, as_of from fx_rates where pair = $1`,
    [pair],
  );
  const row = rows[0];
  if (!row) return null;
  return {
    pair: row.pair,
    rate: Number(row.rate),
    changePercent24h: Number(row.change_percent_24h),
    asOf: row.as_of,
  };
}

/** Persists the drifted rate + a fresh as_of. */
export async function updateRate(pair: string, rate: number): Promise<Date> {
  const { rows } = await query<{ as_of: Date }>(
    `update fx_rates set rate = $2, as_of = now() where pair = $1 returning as_of`,
    [pair, rate],
  );
  return rows[0]!.as_of;
}

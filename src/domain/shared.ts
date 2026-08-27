import { z } from 'zod';
import type { CurrencyCode } from '../contract';

export const CURRENCY_CODES = [
  'NGN',
  'KES',
  'GHS',
  'ZAR',
  'XOF',
  'XAF',
  'EGP',
  'USD',
  'EUR',
  'GBP',
] as const satisfies readonly CurrencyCode[];

export const currencySchema = z.enum(CURRENCY_CODES);

/** Positive integer minor-unit amount. */
export const amountMinorSchema = z.number().int().positive();

export const moneySchema = z.object({
  amountMinor: z.number().int(),
  currency: currencySchema,
});

import { z } from 'zod';
import { config } from '../../config';
import { ApiError } from '../../errors';
import type { FirmQuote } from '../../contract';
import type { RequestSession } from '../../http/auth';
import { getIndicativeRate } from '../fx/service';
import { currencySchema } from '../shared';
import * as repo from './repo';

export const requestFirmQuoteSchema = z.object({
  sendCurrency: currencySchema,
  receiveCurrency: currencySchema,
  amount: z.object({
    amountMinor: z.number().int().positive('Enter an amount greater than zero.'),
    currency: currencySchema,
  }),
  amountField: z.enum(['send', 'receive']),
});

export type RequestFirmQuoteBody = z.infer<typeof requestFirmQuoteSchema>;

/**
 * Both currencies use minor-unit exponent 2, so the minor-unit ratio equals
 * the quoted major-unit rate — same shortcut the frontend mock takes.
 */
function deriveAmounts(input: RequestFirmQuoteBody, rate: number): {
  sendAmountMinor: number;
  receiveAmountMinor: number;
} {
  if (input.amountField === 'send') {
    return {
      sendAmountMinor: input.amount.amountMinor,
      receiveAmountMinor: Math.round(input.amount.amountMinor * rate),
    };
  }
  return {
    sendAmountMinor: Math.round(input.amount.amountMinor / rate),
    receiveAmountMinor: input.amount.amountMinor,
  };
}

export async function requestFirmQuote(
  session: RequestSession,
  input: RequestFirmQuoteBody,
): Promise<FirmQuote> {
  if (input.sendCurrency === input.receiveCurrency) {
    throw ApiError.validation('Send and receive currencies must differ.');
  }
  const expectedCurrency =
    input.amountField === 'send' ? input.sendCurrency : input.receiveCurrency;
  if (input.amount.currency !== expectedCurrency) {
    throw ApiError.validation(
      `Amount currency must be ${expectedCurrency} when entered on the ${input.amountField} side.`,
    );
  }

  const indicative = await getIndicativeRate(input.sendCurrency, input.receiveCurrency);
  const { sendAmountMinor, receiveAmountMinor } = deriveAmounts(input, indicative.rate);

  const stored = await repo.insert({
    customerId: session.customerId,
    sendCurrency: input.sendCurrency,
    receiveCurrency: input.receiveCurrency,
    rate: indicative.rate,
    feeMinor: 0,
    sendAmountMinor,
    receiveAmountMinor,
    ttlSeconds: config.quoteTtlSeconds,
  });

  return stored.firmQuote;
}

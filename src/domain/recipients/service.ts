import { z } from 'zod';
import { withTx } from '../../db/pool';
import { ApiError } from '../../errors';
import { writeAudit } from '../../audit/writeAudit';
import type { Recipient } from '../../contract';
import type { RequestSession } from '../../http/auth';
import { currencySchema } from '../shared';
import * as repo from './repo';
import { stubNameResolver } from './nameResolver';
import type { BankAccountNameResolver } from './nameResolver';

/** The active name resolver. Swap for the payout partner's lookup API. */
export const nameResolver: BankAccountNameResolver = stubNameResolver;

const DEFAULT_BANK_NAME = 'Partner Bank';

export const newRecipientSchema = z.object({
  accountNumber: z.string().trim().min(4, 'Enter the destination account number.'),
  bankCode: z.string().trim().min(1, 'Select the destination bank.'),
  currency: currencySchema,
  country: z.string().trim().length(2, 'Use a 2-letter ISO country code.'),
});

export const saveRecipientSchema = newRecipientSchema.extend({
  accountName: z.string().trim().min(1, 'Confirm the account holder name.'),
});

export type NewRecipientBody = z.infer<typeof newRecipientSchema>;
export type SaveRecipientBody = z.infer<typeof saveRecipientSchema>;

export async function listRecipients(session: RequestSession): Promise<readonly Recipient[]> {
  return repo.listByCustomer(session.customerId);
}

export async function validateBankAccount(
  input: NewRecipientBody,
): Promise<{ accountName: string }> {
  const resolved = await nameResolver.resolve(input);
  if (!resolved) {
    throw ApiError.validation('That account could not be verified. Check the number and bank.');
  }
  return resolved;
}

export async function saveRecipient(
  session: RequestSession,
  input: SaveRecipientBody,
): Promise<Recipient> {
  return withTx(async (client) => {
    const recipient = await repo.insert(client, {
      customerId: session.customerId,
      accountName: input.accountName,
      accountNumber: input.accountNumber,
      bankCode: input.bankCode,
      bankName: DEFAULT_BANK_NAME,
      currency: input.currency,
      country: input.country,
    });
    await writeAudit(client, {
      actorId: session.userId,
      actorRole: session.role,
      action: 'recipient.saved',
      entityType: 'recipient',
      entityId: recipient.id,
      after: recipient,
    });
    return recipient;
  });
}

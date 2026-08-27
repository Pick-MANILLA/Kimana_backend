import { z } from 'zod';
import { config } from '../../config';
import { withTx } from '../../db/pool';
import { ApiError } from '../../errors';
import { writeAudit } from '../../audit/writeAudit';
import type { Transfer, TransferStatus, TransferTimeline } from '../../contract';
import type { RequestSession } from '../../http/auth';
import * as quoteRepo from '../quote/repo';
import * as recipientRepo from '../recipients/repo';
import * as repo from './repo';
import { generateReference } from './reference';
import { advanceToAwaitingFunds, advanceToCompletion } from './engine';

export const createTransferSchema = z.object({
  idempotencyKey: z.string().trim().min(8, 'Provide a stable idempotency key.'),
  quoteId: z.string().trim().min(1),
  recipientId: z.string().trim().min(1),
});

export type CreateTransferBody = z.infer<typeof createTransferSchema>;

const TRANSFER_STATUSES: readonly TransferStatus[] = [
  'CREATED',
  'QUOTED',
  'SCREENED',
  'AWAITING_FUNDS',
  'FUNDED',
  'SETTLING',
  'SETTLED',
  'PAYING_OUT',
  'COMPLETED',
  'REJECTED',
  'EXPIRED',
  'REVERSING',
  'REVERSED',
];

export const transferStatusSchema = z.enum(
  TRANSFER_STATUSES as [TransferStatus, ...TransferStatus[]],
);

async function ownedTransfer(session: RequestSession, id: string): Promise<Transfer> {
  const transfer = await repo.findById(id);
  if (!transfer || transfer.customerId !== session.customerId) {
    throw ApiError.notFound('That transfer couldn’t be found.');
  }
  return transfer;
}

export async function createTransfer(
  session: RequestSession,
  input: CreateTransferBody,
): Promise<Transfer> {
  // Idempotent replay — the client reuses the key verbatim on retry.
  const existing = await repo.findByIdempotencyKey(session.customerId, input.idempotencyKey);
  if (existing) return existing;

  const quote = await quoteRepo.findById(input.quoteId);
  if (!quote || quote.customerId !== session.customerId) {
    throw ApiError.notFound('That quote couldn’t be found.');
  }
  if (quote.expiresAt.getTime() <= Date.now()) {
    throw new ApiError('RATE_EXPIRED', 'That quote has expired. Request a fresh one.', true);
  }

  const recipient = await recipientRepo.findById(input.recipientId, session.customerId);
  if (!recipient) {
    throw ApiError.notFound('That recipient couldn’t be found.');
  }

  const { firmQuote } = quote;

  const transferId = await withTx(async (client) => {
    let id: string;
    try {
      id = await repo.insert(client, {
        reference: generateReference(),
        customerId: session.customerId,
        idempotencyKey: input.idempotencyKey,
        recipientId: recipient.id,
        sendCurrency: firmQuote.sendCurrency,
        receiveCurrency: firmQuote.receiveCurrency,
        sendAmountMinor: firmQuote.breakdown.sendAmount.amountMinor,
        receiveAmountMinor: firmQuote.breakdown.receiveAmount.amountMinor,
        quoteSnapshot: firmQuote,
        initialStatus: 'CREATED',
      });
    } catch (err) {
      // Concurrent create with the same key — unique (customer_id, idempotency_key).
      if ((err as { code?: string }).code === '23505') {
        return null;
      }
      throw err;
    }

    await repo.appendHistory(client, id, 'CREATED');
    await writeAudit(client, {
      actorId: session.userId,
      actorRole: session.role,
      action: 'transfer.created',
      entityType: 'transfer',
      entityId: id,
      after: { status: 'CREATED', quoteId: quote.id, recipientId: recipient.id },
    });
    return id;
  });

  if (transferId === null) {
    const raced = await repo.findByIdempotencyKey(session.customerId, input.idempotencyKey);
    if (raced) return raced;
    throw new ApiError('CONFLICT', 'Could not create the transfer. Try again.', true);
  }

  // Internal checks (quote lock-in, screening) run inline and park the
  // transfer at AWAITING_FUNDS.
  await advanceToAwaitingFunds(transferId);
  scheduleSimulatedProgression(transferId);

  const created = await repo.findById(transferId);
  if (!created) throw new ApiError('SERVER_ERROR', 'Transfer vanished after creation.', true);
  return created;
}

/**
 * Stands in for a collection-partner "funds received" webhook plus the
 * settlement/payout pipeline. -1 disables it (tests drive the engine directly).
 * A process restart between AWAITING_FUNDS and COMPLETED leaves the transfer
 * parked — a real system would resume from a durable queue.
 */
function scheduleSimulatedProgression(transferId: string): void {
  if (config.transferAutoAdvanceMs < 0) return;
  const timer = setTimeout(() => {
    void advanceToCompletion(transferId, { stepDelayMs: config.transferStepDelayMs }).catch(() => {
      // best-effort simulation; real errors surface via audit + status
    });
  }, config.transferAutoAdvanceMs);
  timer.unref?.();
}

export async function getTransfer(session: RequestSession, id: string): Promise<Transfer> {
  return ownedTransfer(session, id);
}

export async function getTimeline(
  session: RequestSession,
  id: string,
): Promise<TransferTimeline> {
  const owner = await repo.getOwnerAndStatus(id);
  if (!owner || owner.customerId !== session.customerId) {
    throw ApiError.notFound('That transfer couldn’t be found.');
  }
  const history = await repo.getHistory(id);
  return {
    transferId: id,
    history,
    isTerminal: repo.timelineIsTerminal(owner.currentStatus),
  };
}

export async function listTransfers(
  session: RequestSession,
  status?: TransferStatus,
): Promise<readonly Transfer[]> {
  return repo.listByCustomer(session.customerId, status);
}

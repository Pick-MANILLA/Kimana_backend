import { randomInt } from 'node:crypto';
import type { PoolClient } from 'pg';
import { withTx } from '../../db/pool';
import { ApiError } from '../../errors';
import { writeAudit } from '../../audit/writeAudit';
import type { CurrencyCode, TransferFailureCategory, TransferStatus } from '../../contract';
import { accountBalanceMinor, getOrCreateAccount, postLedgerEntry } from '../ledger/postEntry';
import * as repo from './repo';
import { FORWARD_STEP, assertTransition } from './stateMachine';
import { isTerminal } from './state';

const REF_ALPHABET = 'ABCDEFGHJKMNPQRSTVWXYZ0123456789';
function tag(prefix: string): string {
  let body = '';
  for (let i = 0; i < 6; i++) body += REF_ALPHABET[randomInt(REF_ALPHABET.length)];
  return `${prefix}-${body}`;
}

interface LockedTransfer {
  id: string;
  reference: string;
  customer_id: string;
  recipient_id: string;
  send_currency: CurrencyCode;
  receive_currency: CurrencyCode;
  send_amount_minor: string;
  receive_amount_minor: string;
  current_status: TransferStatus;
}

async function lockTransfer(client: PoolClient, id: string): Promise<LockedTransfer> {
  const { rows } = await client.query<LockedTransfer>(
    `select id, reference, customer_id, recipient_id, send_currency, receive_currency,
            send_amount_minor, receive_amount_minor, current_status
       from transfers where id = $1 for update`,
    [id],
  );
  const row = rows[0];
  if (!row) throw ApiError.notFound('Transfer not found.');
  return row;
}

async function apply(
  client: PoolClient,
  t: LockedTransfer,
  actorId: string | null,
  to: TransferStatus,
  payload: Record<string, unknown> | null,
): Promise<void> {
  const from = t.current_status;
  assertTransition(from, to);
  await repo.appendHistory(client, t.id, to, payload);
  await repo.setStatus(client, t.id, to);
  await postLedgerFor(client, t, to);
  await writeAudit(client, {
    actorId,
    actorRole: actorId ? 'customer' : null,
    action: 'transfer.state_change',
    entityType: 'transfer',
    entityId: t.id,
    before: { status: from },
    after: { status: to, ...(payload ?? {}) },
  });
  t.current_status = to;
}

/**
 * Customer-account-centric ledger model for the FX-through payment:
 *   FUNDED     -sendAmount   from the send-currency account
 *   SETTLED    +receiveAmount to the receive-currency account
 *   COMPLETED  -receiveAmount from the receive-currency account (paid to beneficiary)
 * Net effect on the customer: -sendAmount in the send currency.
 */
async function postLedgerFor(
  client: PoolClient,
  t: LockedTransfer,
  to: TransferStatus,
): Promise<void> {
  const sendMinor = Number(t.send_amount_minor);
  const receiveMinor = Number(t.receive_amount_minor);

  if (to === 'FUNDED') {
    const accountId = await getOrCreateAccount(client, t.customer_id, t.send_currency);
    await postLedgerEntry(client, {
      accountId,
      transferId: t.id,
      amountMinor: -sendMinor,
      currency: t.send_currency,
      description: `Transfer ${t.reference} — funded`,
    });
    return;
  }

  if (to === 'SETTLED') {
    const accountId = await getOrCreateAccount(client, t.customer_id, t.receive_currency);
    await postLedgerEntry(client, {
      accountId,
      transferId: t.id,
      amountMinor: receiveMinor,
      currency: t.receive_currency,
      description: `Transfer ${t.reference} — converted`,
    });
    return;
  }

  if (to === 'COMPLETED') {
    const accountId = await getOrCreateAccount(client, t.customer_id, t.receive_currency);
    const { rows } = await client.query<{ account_name: string }>(
      `select account_name from recipients where id = $1`,
      [t.recipient_id],
    );
    const beneficiary = rows[0]?.account_name ?? 'beneficiary';
    await postLedgerEntry(client, {
      accountId,
      transferId: t.id,
      amountMinor: -receiveMinor,
      currency: t.receive_currency,
      description: `Transfer ${t.reference} — paid to ${beneficiary}`,
    });
  }
}

function payloadFor(to: TransferStatus): Record<string, unknown> | null {
  switch (to) {
    case 'SCREENED':
      return { hold: false }; // screening decisions are P3
    case 'AWAITING_FUNDS':
      return { fundingReference: tag('FR') };
    case 'COMPLETED':
      return { payoutReference: tag('PO') };
    default:
      return null;
  }
}

/** Applies exactly one transition. Returns the resulting status (unchanged if blocked). */
export async function advanceOnce(transferId: string, actorId: string | null = null): Promise<TransferStatus> {
  return withTx(async (client) => {
    const t = await lockTransfer(client, transferId);
    const from = t.current_status;
    const to = FORWARD_STEP[from];
    if (!to) return from;

    if (from === 'AWAITING_FUNDS') {
      const sendAccountId = await getOrCreateAccount(client, t.customer_id, t.send_currency);
      const balance = await accountBalanceMinor(client, sendAccountId);
      if (balance < Number(t.send_amount_minor)) {
        await apply(client, t, actorId, 'REJECTED', {
          failureCategory: 'validation' satisfies TransferFailureCategory,
          reasonCode: 'INSUFFICIENT_FUNDS',
        });
        return 'REJECTED';
      }
    }

    await apply(client, t, actorId, to, payloadFor(to));
    return to;
  });
}

const PRE_FUNDING: ReadonlySet<TransferStatus> = new Set(['CREATED', 'QUOTED', 'SCREENED']);

async function statusOf(id: string): Promise<TransferStatus> {
  const owner = await repo.getOwnerAndStatus(id);
  if (!owner) throw ApiError.notFound('Transfer not found.');
  return owner.currentStatus;
}

/** Drives a just-created transfer through the internal checks to AWAITING_FUNDS. */
export async function advanceToAwaitingFunds(id: string): Promise<TransferStatus> {
  let status = await statusOf(id);
  while (PRE_FUNDING.has(status)) {
    const next = await advanceOnce(id);
    if (next === status) break;
    status = next;
    if (isTerminal(status)) break;
  }
  return status;
}

const wait = (ms: number) => (ms > 0 ? new Promise<void>((r) => setTimeout(r, ms)) : Promise.resolve());

/** Drives a transfer forward until terminal or no forward step remains. */
export async function advanceToCompletion(
  id: string,
  opts: { stepDelayMs?: number } = {},
): Promise<TransferStatus> {
  let status = await statusOf(id);
  while (FORWARD_STEP[status]) {
    await wait(opts.stepDelayMs ?? 0);
    const next = await advanceOnce(id);
    if (next === status) break;
    status = next;
    if (isTerminal(status)) break;
  }
  return status;
}

/** Pre-funding timeout. No ledger effect — no funds have moved. */
export async function expireTransfer(id: string, actorId: string | null = null): Promise<void> {
  await withTx(async (client) => {
    const t = await lockTransfer(client, id);
    await apply(client, t, actorId, 'EXPIRED', null);
  });
}

/**
 * Post-completion unwind: REVERSING → REVERSED with one compensating entry
 * (+sendAmount back to the send account). Only valid from COMPLETED, where the
 * receive-side entries have already netted to zero.
 */
export async function reverseTransfer(
  id: string,
  reason: string,
  actorId: string | null = null,
): Promise<{ reversalLedgerEntryId: string }> {
  return withTx(async (client) => {
    const t = await lockTransfer(client, id);
    await apply(client, t, actorId, 'REVERSING', { reason });

    const fundingEntry = await client.query<{ id: string }>(
      `select id from ledger_entries
        where transfer_id = $1 and amount_minor < 0 and description like '%funded%'
        order by posted_at limit 1`,
      [t.id],
    );

    const sendAccountId = await getOrCreateAccount(client, t.customer_id, t.send_currency);
    const { entryId } = await postLedgerEntry(client, {
      accountId: sendAccountId,
      transferId: t.id,
      amountMinor: Number(t.send_amount_minor),
      currency: t.send_currency,
      description: `Transfer ${t.reference} — reversed`,
      ...(fundingEntry.rows[0] ? { reversalOfEntryId: fundingEntry.rows[0].id } : {}),
    });

    await apply(client, t, actorId, 'REVERSED', { reason, reversalLedgerEntryId: entryId });
    return { reversalLedgerEntryId: entryId };
  });
}

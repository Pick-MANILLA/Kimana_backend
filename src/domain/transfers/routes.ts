import type { FastifyInstance } from 'fastify';
import { z } from 'zod';
import {
  createTransfer,
  createTransferSchema,
  getTimeline,
  getTransfer,
  listTransfers,
  transferStatusSchema,
} from './service';

const listQuerySchema = z.object({ status: transferStatusSchema.optional() });

export async function transferRoutes(app: FastifyInstance): Promise<void> {
  app.post('/transfers', async (req, reply) => {
    const idempotencyHeader = req.headers['idempotency-key'];
    const raw = (req.body ?? {}) as Record<string, unknown>;
    const body = createTransferSchema.parse({
      idempotencyKey:
        typeof idempotencyHeader === 'string' ? idempotencyHeader : raw.idempotencyKey,
      quoteId: raw.quoteId,
      recipientId: raw.recipientId,
    });
    const transfer = await createTransfer(req.session, body);
    reply.status(201).send(transfer);
  });

  app.get<{ Params: { id: string } }>('/transfers/:id', async (req) => {
    return getTransfer(req.session, req.params.id);
  });

  app.get<{ Params: { id: string } }>('/transfers/:id/timeline', async (req) => {
    return getTimeline(req.session, req.params.id);
  });

  app.get('/transfers', async (req) => {
    const { status } = listQuerySchema.parse(req.query);
    return listTransfers(req.session, status);
  });
}

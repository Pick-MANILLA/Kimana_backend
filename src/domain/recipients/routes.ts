import type { FastifyInstance } from 'fastify';
import {
  listRecipients,
  newRecipientSchema,
  saveRecipient,
  saveRecipientSchema,
  validateBankAccount,
} from './service';

export async function recipientRoutes(app: FastifyInstance): Promise<void> {
  app.get('/recipients', async (req) => {
    return listRecipients(req.session);
  });

  app.post('/recipients/validate', async (req) => {
    const input = newRecipientSchema.parse(req.body);
    return validateBankAccount(input);
  });

  app.post('/recipients', async (req, reply) => {
    const input = saveRecipientSchema.parse(req.body);
    const recipient = await saveRecipient(req.session, input);
    reply.status(201).send(recipient);
  });
}

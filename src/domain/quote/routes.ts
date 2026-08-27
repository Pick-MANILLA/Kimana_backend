import type { FastifyInstance } from 'fastify';
import { requestFirmQuote, requestFirmQuoteSchema } from './service';

export async function quoteRoutes(app: FastifyInstance): Promise<void> {
  app.post('/quotes', async (req, reply) => {
    const input = requestFirmQuoteSchema.parse(req.body);
    const quote = await requestFirmQuote(req.session, input);
    reply.status(201).send(quote);
  });
}

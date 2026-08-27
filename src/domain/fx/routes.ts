import type { FastifyInstance } from 'fastify';
import { z } from 'zod';
import { currencySchema } from '../shared';
import { getIndicativeRate } from './service';

const querySchema = z.object({
  send: currencySchema,
  receive: currencySchema,
});

export async function fxRoutes(app: FastifyInstance): Promise<void> {
  app.get('/rates/indicative', async (req) => {
    const { send, receive } = querySchema.parse(req.query);
    return getIndicativeRate(send, receive);
  });
}

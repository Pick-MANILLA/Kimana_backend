import type { FastifyInstance } from 'fastify';
import { ZodError } from 'zod';
import { ApiError } from '../errors';

/**
 * Single error contract for every route: `{ code, message, retryable }` with
 * the status from docs/backend-plan.md §02. Matches what the frontend's
 * ApiError consumers switch on.
 */
export function registerErrorHandler(app: FastifyInstance): void {
  app.setErrorHandler((err, req, reply) => {
    if (err instanceof ApiError) {
      reply
        .status(err.httpStatus)
        .send({ code: err.code, message: err.message, retryable: err.retryable });
      return;
    }

    if (err instanceof ZodError) {
      reply.status(400).send({
        code: 'VALIDATION',
        message: err.issues[0]?.message ?? 'The request failed validation.',
        retryable: false,
      });
      return;
    }

    // @fastify/multipart: file exceeded the configured limit.
    if ((err as { code?: string }).code === 'FST_REQ_FILE_TOO_LARGE') {
      reply.status(400).send({
        code: 'VALIDATION',
        message: 'That file is larger than 10 MB. Compress it or choose a smaller copy.',
        retryable: false,
      });
      return;
    }

    // Fastify schema validation (JSON body/query).
    if ((err as { validation?: unknown }).validation) {
      reply
        .status(400)
        .send({ code: 'VALIDATION', message: (err as Error).message, retryable: false });
      return;
    }

    req.log.error({ err }, 'unhandled error');
    reply.status(500).send({
      code: 'SERVER_ERROR',
      message: 'Something went wrong on our end. Try again in a moment.',
      retryable: true,
    });
  });
}

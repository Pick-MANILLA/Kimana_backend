import type { FastifyInstance } from 'fastify';
import { ApiError } from '../../errors';
import type { OnboardingDocumentType } from '../../contract';
import {
  saveBusinessBodySchema,
  savePrincipalsBodySchema,
  submitBodySchema,
} from './schemas';
import * as service from './service';

const DOCUMENT_TYPES: ReadonlySet<string> = new Set([
  'cac_certificate',
  'memart',
  'proof_of_address',
  'directors_id',
  'board_resolution',
]);

export async function onboardingRoutes(app: FastifyInstance): Promise<void> {
  app.get('/onboarding/application', async (req) => {
    return service.getApplication(req.session);
  });

  app.put('/onboarding/application/business', async (req) => {
    const body = saveBusinessBodySchema.parse(req.body);
    return service.saveBusinessDetails(req.session, body.business, body.applicationId);
  });

  app.put('/onboarding/application/principals', async (req) => {
    const body = savePrincipalsBodySchema.parse(req.body);
    return service.savePrincipals(req.session, body.principals, body.applicationId);
  });

  app.post('/onboarding/application/submit', async (req) => {
    const body = submitBodySchema.parse(req.body ?? {});
    return service.submit(req.session, body?.applicationId);
  });

  app.post('/onboarding/application/documents', async (req) => {
    const parts = req.parts();
    let fileBuffer: Buffer | undefined;
    let fileName = '';
    let mimeType = '';
    let type: string | undefined;
    let applicationId: string | undefined;

    for await (const part of parts) {
      if (part.type === 'file') {
        fileBuffer = await part.toBuffer();
        fileName = part.filename;
        mimeType = part.mimetype;
      } else if (part.fieldname === 'type') {
        type = String(part.value);
      } else if (part.fieldname === 'applicationId') {
        applicationId = String(part.value);
      }
    }

    if (!type || !DOCUMENT_TYPES.has(type)) {
      throw ApiError.validation('Provide a valid document `type`.');
    }
    if (!fileBuffer) {
      throw ApiError.validation('No file was included in the upload.');
    }

    return service.uploadDocument(
      req.session,
      { type: type as OnboardingDocumentType, fileName, mimeType, buffer: fileBuffer },
      applicationId,
    );
  });

  app.post<{ Params: { id: string } }>(
    '/onboarding/application/documents/:id/retry',
    async (req) => {
      return service.retryDocumentUpload(req.session, req.params.id);
    },
  );

  app.delete<{ Params: { id: string } }>(
    '/onboarding/application/documents/:id',
    async (req, reply) => {
      await service.removeDocument(req.session, req.params.id);
      reply.status(204).send();
    },
  );
}

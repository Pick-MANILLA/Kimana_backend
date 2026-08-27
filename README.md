# Kimana_backend

Backend for Kimana, implementing the typed `ApiClient` contract that
`Kimana_frontend` already defined and runs against. Full plan and rationale:
[`docs/backend-plan.md`](docs/backend-plan.md).

**Status: P1 slice.** Auth session, the onboarding wizard, and the dashboard
overview are served for real; everything else (transfers, quotes, ledger
postings, screening, trade documents, all `/ops`) is still ahead — see the plan.

## Stack

TypeScript · Fastify 5 · Postgres · Zod · Vitest. No ORM — plain SQL migrations
(`migrations/*.sql`) and `pg`. Money is always integer minor units + a currency
code. `ledger_entries` and `audit_log` are append-only (DB triggers).

## Quickstart

```bash
cp .env.example .env
docker compose up -d          # Postgres on :5432
npm install
npm run migrate               # apply migrations/*.sql
npm run seed                   # demo customer "Chinonso" / Adunola Exports Ltd
npm run dev                    # http://localhost:4000  (also runs migrations)
```

```bash
npm test                       # integration suite (needs Postgres up)
npm run build                  # tsc -> dist/
npm run typecheck
```

## Endpoints (this slice)

| Method | Path | |
|---|---|---|
| GET | `/health` | unauthenticated |
| GET | `/session` | seeded demo session |
| GET | `/onboarding/application` | |
| PUT | `/onboarding/application/business` | `{ business, applicationId? }` |
| PUT | `/onboarding/application/principals` | `{ principals, applicationId? }` |
| POST | `/onboarding/application/documents` | multipart: `type`, `file` |
| POST | `/onboarding/application/documents/:id/retry` | |
| DELETE | `/onboarding/application/documents/:id` | 204 |
| POST | `/onboarding/application/submit` | synchronous approve for the slice |
| GET | `/dashboard/overview` | balances summed from `ledger_entries`; stats/actions seeded |

Errors are `{ code, message, retryable }` with the status mapping in
`docs/backend-plan.md` §02.

## Layout

```
migrations/            forward-only SQL, run in order
src/
  contract/            vendored mirror of Kimana_frontend/src/api/types (see its README)
  config.ts  errors.ts
  db/                  pool + withTx + migration runner
  http/                error handler, session middleware
  audit/               writeAudit (same-transaction append)
  money/               server-side Money helpers
  storage/             DocumentStore (filesystem impl; MinIO/S3 later)
  domain/
    onboarding/        schemas · repo · service · routes
    dashboard/         service · routes · placeholder content
    ledger/            balance reads
  seed/                demo tenant, mirrors the frontend mock store
test/                  vitest integration tests (app.inject + real Postgres)
integration/           drop-in live client + wiring notes for Kimana_frontend
```

## Connecting the frontend

See [`integration/README.md`](integration/README.md) — copy one file into
`Kimana_frontend`, flip `src/api/index.ts`, set `VITE_API_URL`.

## Known gaps (tracked in docs/backend-plan.md)

- Onboarding `submit` approves synchronously and always succeeds — async KYB and
  the `rejected` path are next.
- Auth is a single seeded session; no login yet.
- `src/contract` is a vendored copy, not a shared `@kimana/contract` package.
- Dev-only `npm audit` findings in the vitest/vite/esbuild chain (dev server
  SSRF); not in the runtime dependency tree.

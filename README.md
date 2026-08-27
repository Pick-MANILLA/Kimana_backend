# Kimana_backend

Backend for Kimana, implementing the typed `ApiClient` contract that
`Kimana_frontend` already defined and runs against. Full plan and rationale:
[`docs/backend-plan.md`](docs/backend-plan.md).

**Status: P1 + P2.** Auth session, the onboarding wizard, the dashboard
overview, indicative FX, recipients, firm quotes, and the full transfer
lifecycle (state machine + ledger postings) are served for real. Still ahead:
trade documents, customer screening view, all `/ops` — see the plan.

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

## Endpoints

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
| POST | `/onboarding/application/submit` | walks `draft→submitted→in_review`, runs the KYB provider, lands on `approved` or `rejected` |
| GET | `/dashboard/overview` | balances from `ledger_entries`; in-progress count + USD 30d volume computed from transfers |
| GET | `/rates/indicative?send=&receive=` | drifting stub rate |
| GET | `/recipients` · POST `/recipients/validate` · POST `/recipients` | |
| POST | `/quotes` | firm quote, `expiresAt = issuedAt + 90s` |
| POST | `/transfers` | idempotent (`Idempotency-Key` header or body); parks at `AWAITING_FUNDS` |
| GET | `/transfers` · `/transfers/:id` · `/transfers/:id/timeline` | `?status=` filter on the list |

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
    onboarding/        schemas · repo · service · routes · kyb/ (provider + stub)
    dashboard/         service · routes · placeholder content
    fx/                indicative rates (FxProvider stub)
    recipients/        list / validate (name-resolver stub) / save
    quote/             firm quotes
    transfers/         createTransfer · reads · stateMachine · engine
    ledger/            balance reads · postEntry (running-balance under a lock)
  seed/                demo tenant, mirrors the frontend mock store
test/                  vitest integration tests (app.inject + real Postgres)
integration/           drop-in live client + wiring notes for Kimana_frontend
```

## Connecting the frontend

See [`integration/README.md`](integration/README.md) — copy one file into
`Kimana_frontend`, flip `src/api/index.ts`, set `VITE_API_URL`.

## KYB (onboarding `submit`)

`submit` moves the application through `submitted → in_review`, runs
`kybProvider.runChecks()`, then commits `approved` (+ `approvedSummary`) or
`rejected` (+ `rejectionReasons`). Per-check results land in `kyb_checks`.

The request stays open until a terminal status — matching the contract and the
frontend, which doesn't poll. The active provider is a **stub**
(`src/domain/onboarding/kyb/stubProvider.ts`): everything passes unless the data
trips a documented trigger — legal name containing `REJECT` (CAC), a principal
BVN of `00000000000` (NIBSS), a principal name containing `SANCTION`. Swap
`kyb/index.ts` for a real provider. `KYB_CHECK_DELAY_MS` tunes the simulated
latency (0 in tests).

> The frontend has no `rejected` screen yet — `VerificationPage` navigates to
> `/onboarding/approved` on any resolved submit, so a rejection currently shows
> as a stuck "Loading your account…". Building that screen is frontend work.

## Transfer lifecycle

`createTransfer` snapshots the quote (rejecting an expired one with
`RATE_EXPIRED`), then runs the internal checks inline and parks the transfer at
`AWAITING_FUNDS` with a funding reference. `TRANSFER_AUTO_ADVANCE_MS` later
simulates the collection-partner "funds received" webhook plus the
settlement/payout pipeline, driving it to `COMPLETED` (`-1` disables; tests
call the engine directly).

`src/domain/transfers/stateMachine.ts` holds the explicit transition table —
every state change goes through `assertTransition`. `engine.ts` applies one
transition per transaction (history row + audit + ledger postings together).

**Ledger model** (customer-account-centric FX-through payment):

| Transition | Posting |
|---|---|
| `→ FUNDED` | `-sendAmount` from the send-currency account (rejects `INSUFFICIENT_FUNDS` if short) |
| `→ SETTLED` | `+receiveAmount` to the receive-currency account |
| `→ COMPLETED` | `-receiveAmount` from the receive-currency account (paid to the beneficiary) |
| `→ REVERSED` | `+sendAmount` back to the send account, linked via `reversal_of_entry_id` |

`running_balance_minor` is computed under an account row lock so concurrent
postings serialise.

## Known gaps (tracked in docs/backend-plan.md)

- Auth is a single seeded session; no login yet.
- `src/contract` is a vendored copy, not a shared `@kimana/contract` package.
- KYB runs inside the request; a minutes-long real provider would want
  submit + webhook/poll instead.
- Transfer progression is an in-process `setTimeout`; a process restart between
  `AWAITING_FUNDS` and `COMPLETED` leaves the transfer parked (no resume sweep).
- Screening always clears (`hold: false`) — the ops decision path is P3/P4.
- `payoutSuccessRatePercent` / `avgSettlementSeconds` on the dashboard are still
  placeholder (need settlement-timing data).
- Dev-only `npm audit` findings in the vitest/vite/esbuild chain (dev server
  SSRF); not in the runtime dependency tree.

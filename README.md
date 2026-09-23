# Kimana_backend

Backend for Kimana, implementing the typed `ApiClient` contract that
`Kimana_frontend` already defined and runs against. Full plan and rationale:
`docs/backend-plan.md` (not tracked).

**Status: P1 + P2.** Auth session, the onboarding wizard, the dashboard
overview, indicative FX, recipients, firm quotes, and the full transfer
lifecycle (state machine + ledger postings) are served for real. Still ahead:
trade documents, customer screening view, all `/ops`.

## Stack

Rust · axum 0.8 · sqlx 0.8 (Postgres, runtime queries, no ORM) · tokio.
Plain SQL migrations in `migrations/`. Money is always integer minor units
(`i64`) + a currency code. `ledger_entries` and `audit_log` are append-only
(DB triggers).

## Quickstart

```bash
cp .env.example .env
docker compose up -d           # Postgres on :5432
cargo run --bin migrate        # apply migrations/*.sql
cargo run --bin seed           # demo customer "Chinonso" / Adunola Exports Ltd
cargo run                      # http://localhost:4000  (also runs migrations)
```

```bash
cargo test                     # integration suite (needs Postgres up)
cargo build --release
cargo clippy --all-targets
```

Tests share one database and reseed per test, serialised with a
cross-process file lock (`serial_test`), so `cargo test` is safe to run as-is.

## Endpoints

| Method | Path | |
|---|---|---|
| GET | `/health` | unauthenticated |
| GET | `/session` | seeded demo session |
| GET | `/onboarding/application` | |
| PUT | `/onboarding/application/business` | `{ business, applicationId? }` |
| PUT | `/onboarding/application/principals` | `{ principals, applicationId? }` |
| POST | `/onboarding/application/documents` | multipart: `type`, `file` |
| POST | `/onboarding/application/documents/{id}/retry` | |
| DELETE | `/onboarding/application/documents/{id}` | 204 |
| POST | `/onboarding/application/submit` | walks `draft→submitted→in_review`, runs the KYB provider, lands on `approved` or `rejected` |
| GET | `/dashboard/overview` | balances from `ledger_entries`; in-progress count + USD 30d volume from transfers |
| GET | `/rates/indicative?send=&receive=` | drifting stub rate |
| GET/POST | `/recipients` · POST `/recipients/validate` | |
| POST | `/quotes` | firm quote, `expiresAt = issuedAt + 90s` |
| POST | `/transfers` | idempotent (`Idempotency-Key` header or body); parks at `AWAITING_FUNDS` |
| GET | `/transfers` · `/transfers/{id}` · `/transfers/{id}/timeline` | `?status=` filter on the list |

Errors are `{ code, message, retryable }` (SCREAMING_SNAKE codes) with the
status mapping in `docs/backend-plan.md` §02.

## Layout

```
abi/                   SettlementVault ABI, copied from kimana_contract
migrations/            forward-only SQL, embedded via sqlx::migrate!
src/
  main.rs              server entrypoint (migrate + serve)
  bin/{migrate,seed}.rs
  lib.rs               module tree + build_app(state) -> Router
  config.rs  error.rs  db.rs
  http/                Session extractor (FromRequestParts), Body<T> JSON extractor
  audit.rs             write_audit (same-transaction append)
  storage.rs           filesystem document store (MinIO/S3 later)
  contract/            serde structs mirroring Kimana_frontend/src/api/types
  domain/
    onboarding/        schema · repo · service · routes · kyb (stub provider)
    dashboard.rs
    fx.rs              indicative rates (stub jitters a seeded rate)
    recipients.rs      list / validate (name-resolver stub) / save
    quote.rs           firm quotes
    transfers/         service · repo · state_machine · engine · routes
    ledger.rs          balance reads · post_entry (running balance under a lock)
  settlement/          alloy client for the SettlementVault: bindings · units · client · error
  seed.rs              demo tenant, mirrors the frontend mock store
tests/                 integration tests — tower::oneshot against the real Router + Postgres
integration/           drop-in live client + wiring notes for Kimana_frontend
scripts/               settlement-e2e.sh: Anvil + LocalE2E.s.sol + tests/settlement_anvil.rs
```

## Contract fidelity

`src/contract/` is a hand-maintained mirror of `Kimana_frontend/src/api/types`.
Rust and TypeScript don't share a compiler, so keeping serde field names
(`#[serde(rename_all = "camelCase")]`) and enum reprs faithful to the TS
contract is a **review discipline**, not a compile-time guarantee. A future
step is to generate these from the TS types (or an OpenAPI spec) — see
`docs/backend-plan.md` §00.

## KYB (onboarding `submit`)

`submit` moves the application through `submitted → in_review`, runs
`kyb::run_checks`, then commits `approved` (+ `approvedSummary`) or `rejected`
(+ `rejectionReasons`). Per-check results land in `kyb_checks`. It resolves
only at a terminal status — the frontend does not poll.

The provider is a **stub** (`src/domain/onboarding/kyb.rs`): everything passes
unless the data trips a documented trigger — legal name containing `reject`
(CAC), a principal BVN of `00000000000` (NIBSS), a principal name containing
`sanction`. `KYB_CHECK_DELAY_MS` tunes the simulated latency (0 in tests).

> The frontend has no `rejected` screen yet — `VerificationPage` navigates to
> `/onboarding/approved` on any resolved submit. Building that screen is
> frontend work.

## Transfer lifecycle

`create_transfer` snapshots the quote (rejecting an expired one with
`RATE_EXPIRED`), then runs the internal checks inline and parks the transfer at
`AWAITING_FUNDS` with a funding reference. `TRANSFER_AUTO_ADVANCE_MS` later
spawns a task that simulates the collection-partner "funds received" webhook
plus the settlement/payout pipeline, driving it to `COMPLETED` (`-1` disables;
tests call the engine directly).

`src/domain/transfers/state_machine.rs` holds the explicit transition table —
every change goes through `assert_transition` (illegal → `CONFLICT`).
`engine.rs` applies one transition per transaction (history row + audit +
ledger postings together).

**Ledger model** (customer-account-centric FX-through payment; running balance
computed under an account row lock):

| Transition | Posting |
|---|---|
| `→ FUNDED` | `-sendAmount` from the send-currency account (rejects `INSUFFICIENT_FUNDS` if short) |
| `→ SETTLED` | `+receiveAmount` to the receive-currency account |
| `→ COMPLETED` | `-receiveAmount` from the receive-currency account (paid to the beneficiary) |
| `→ REVERSED` | `+sendAmount` back to the send account, linked via `reversal_of_entry_id` |

## On-chain settlement

`src/settlement/` calls the SettlementVault from
[kimana_contract](https://github.com/Pick-MANILLA/kimana_contract) with alloy.
Bindings come from `abi/SettlementVault.json`, a copy of the contract repo's
file: copy it again whenever the contract's ABI changes.

- `units`: `transfer_ref`, `quote_id`, `cents_to_usdc` / `usdc_to_cents`,
  `RateE8` (8-decimal integer rate, parsed from a decimal string) and
  `receive_amount_minor`. Integer only, and the parity tests pin them to the
  contract's own test vectors.
- `client`: `lock_quote`, `settle`, `cancel_quote`, `refund`. Each waits for
  the receipt. `lock_quote` derives the counterparty amount with the vault's
  formula and refuses to send an already-expired quote.
- `error`: vault reverts decoded and mapped to API errors. An expired quote is
  `RATE_EXPIRED`; reuse or a wrong lifecycle state is `CONFLICT`; a blocked
  divergent rate or a paused vault is `PARTNER_FAILURE`; a limit breach is
  `COMPLIANCE_HOLD`; a quote the vault rejects as malformed is a logged
  `SERVER_ERROR`.

Config: `SETTLEMENT_RPC_URL` and `SETTLEMENT_VAULT_ADDRESS`. The operator
signer is passed to `SettlementClient::new` and never read from the
environment.

Backend-driven lock, settle, cancel and refund against the contract's
`make e2e` Anvil setup (needs Foundry and `make install` in the contract repo):

```bash
KIMANA_CONTRACT_DIR=../kimana_contract bash scripts/settlement-e2e.sh
```

Not wired into the transfer lifecycle yet: `quotes.rate` is still stored as
`f64`, and moving it to `RateE8` is its own change.

## Connecting the frontend

See `integration/README.md` — copy one file into `Kimana_frontend`, flip
`src/api/index.ts`, set `NEXT_PUBLIC_API_URL`. The wire contract (HTTP/JSON) is
identical regardless of backend language.

## Known gaps (tracked in docs/backend-plan.md)

- Auth is a single seeded session; no login yet.
- `src/contract` is a hand-maintained mirror, not generated from the TS types.
- KYB + transfer progression run in-process; a restart between `AWAITING_FUNDS`
  and `COMPLETED` leaves the transfer parked (no resume sweep).
- Screening always clears (`hold: false`) — the ops decision path is P3/P4.
- `payoutSuccessRatePercent` / `avgSettlementSeconds` on the dashboard are still
  placeholder.

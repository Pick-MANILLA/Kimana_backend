# Kimana Backend — Issue Tracker

50 issues grouped by priority. Each issue contains a description of the problem, the exact files involved, a step-by-step resolution guide, and testable acceptance criteria.

**Labels:** `bug` | `enhancement` | `tech-debt`

---

## P1 — Core Correctness

---

### ISSUE-01: Session extractor always resolves to the hardcoded demo user

**Priority:** P1
**Label:** bug
**Files:** `src/http/auth.rs`, `src/ids.rs`

#### What the issue is

`Session::from_request_parts` in `src/http/auth.rs` ignores every header on the incoming request and unconditionally queries the database for the row where `u.id = DEMO_USER_ID` (`00000000-0000-4000-8000-000000000001`). There is no credential check, no cookie read, no JWT validation. Any caller — authenticated or not — gets the same Chinonso / Adunola Exports Ltd session. This means:

- There is no way for a second business to sign up.
- An unauthenticated request to any protected route succeeds silently.
- The login page on the frontend is purely cosmetic; hitting "Login" just redirects to `/dashboard` with no token exchange.

The comment in the file acknowledges this: `"P1 slice: a single seeded demo customer, looked up by DEMO_USER_ID. Real login is P1-later — only this extractor changes."` That later work has not landed.

#### How to fix it — step by step

1. **Design the credential model.** Decide whether to use signed session cookies (simpler, works with axum-extra's `CookieJar`) or short-lived JWT bearer tokens (more frontend-friendly for Next.js). For a B2B product a secure HttpOnly cookie with a server-side session table is the safest starting point.

2. **Add a `sessions` table to the database.** Create a new migration file `migrations/006_sessions.sql`:
   ```sql
   create table sessions (
     id          uuid primary key default gen_random_uuid(),
     user_id     uuid not null references users(id) on delete cascade,
     token_hash  text not null unique,   -- bcrypt or sha256 of the opaque token
     created_at  timestamptz not null default now(),
     expires_at  timestamptz not null,
     revoked_at  timestamptz
   );
   create index sessions_token_hash_idx on sessions(token_hash);
   ```

3. **Add a `password_hash` column to `users`.** Alter the users table (new migration or extend migration 006) to add `password_hash text`. Hash passwords with `bcrypt` via the `bcrypt` crate.

4. **Add a `POST /auth/login` route.** In `src/routes.rs` (or a new `src/domain/auth/`) add a handler that:
   - Accepts `{ email, password }` JSON.
   - Looks up the user by email.
   - Verifies the password hash with `bcrypt::verify`.
   - On success, generates a cryptographically random opaque token (`rand::thread_rng().fill_bytes`), stores its SHA-256 hash in `sessions`, and returns the raw token in a `Set-Cookie: session=<token>; HttpOnly; Secure; SameSite=Strict` header (or as a JSON bearer token).
   - Returns `401 UNAUTHORIZED` with `ErrorCode::Unauthorized` on failure.

5. **Rewrite `Session::from_request_parts`.** Instead of binding `DEMO_USER_ID`, read the session token from the `Cookie` header (or `Authorization: Bearer`), hash it, look it up in the `sessions` table with `expires_at > now() and revoked_at is null`, and join to `users` and `customers`. Return `ApiError::unauthorized` if nothing matches.

6. **Add a `POST /auth/logout` route** that sets `revoked_at = now()` on the session row.

7. **Update the seed** (`src/seed.rs`) to insert a `password_hash` for the demo user so the login flow works end-to-end in development.

8. **Update the frontend** (`Kimana_frontend/src/api/mock/`) to call the real login endpoint instead of the mock, and store/send the session token on subsequent requests.

#### Acceptance criteria

- [ ] `POST /auth/login` with correct credentials returns a session token and `200 OK`.
- [ ] `POST /auth/login` with wrong credentials returns `401`.
- [ ] All protected routes return `401` when called without a valid session token.
- [ ] `POST /auth/logout` invalidates the session; subsequent requests with that token return `401`.
- [ ] The demo user (`Chinonso`) can log in using `cargo run --bin seed`-inserted credentials.
- [ ] No route relies on `DEMO_USER_ID` at runtime after this change.

---

### ISSUE-02: CORS origin hardcoded to Vite port 5173; frontend runs on Next.js port 3000

**Priority:** P1
**Label:** bug
**Files:** `src/config.rs`, `.env.example`

#### What the issue is

`Config::from_env` in `src/config.rs` defaults `cors_origin` to `"http://localhost:5173"`:

```rust
cors_origin: var("CORS_ORIGIN", "http://localhost:5173"),
```

The frontend (`Kimana_frontend`) is a Next.js 16 app that runs on port `3000` by default. Any browser request from the frontend to the backend currently fails with a CORS preflight rejection (`Access-Control-Allow-Origin` mismatch). The only way it works today is if a developer manually sets `CORS_ORIGIN=http://localhost:3000` in their environment, but `.env.example` still shows the wrong default and the README does not mention this.

This is a silent breakage: the backend starts without error, all tests pass (they bypass CORS), but the actual browser integration is broken out of the box.

#### How to fix it — step by step

1. **Change the default** in `src/config.rs`:
   ```rust
   cors_origin: var("CORS_ORIGIN", "http://localhost:3000"),
   ```

2. **Update `.env.example`** to reflect the correct default:
   ```
   CORS_ORIGIN=http://localhost:3000
   ```

3. **Update `README.md`** to document the `CORS_ORIGIN` variable and note that it must match the frontend's origin in staging/production.

4. **Consider supporting a comma-separated list** of origins (useful for running both ports during transition). If needed, update `build_app` in `src/lib.rs` to parse `CORS_ORIGIN` as a comma-split list and pass each to `CorsLayer::allow_origin` using `tower_http::cors::AllowOrigin::list`.

#### Acceptance criteria

- [ ] Fresh `cargo run` with no `.env` overrides allows browser requests from `http://localhost:3000`.
- [ ] `.env.example` shows `CORS_ORIGIN=http://localhost:3000`.
- [ ] The frontend's login and dashboard pages can reach the backend without CORS errors in a browser.

---

### ISSUE-03: `trade_description` from the HTTP request body is never read

**Priority:** P1
**Label:** bug
**Files:** `src/domain/transfers/service.rs`, `src/domain/transfers/routes.rs`

#### What the issue is

The `CreateTransferBody` struct in `src/domain/transfers/routes.rs` does not include a `trade_description` field. Even if the frontend sends one, it is silently dropped by serde before reaching the service layer. In `service.rs`, `InsertTransfer` is built with `trade_description: None` unconditionally:

```rust
InsertTransfer {
    ...
    trade_description: None,   // <- always None, body field never wired up
    ...
}
```

The `transfers` table has a `trade_description text` column and the `Transfer` contract struct has a matching `trade_description: Option<String>` field — the plumbing exists but the handler never feeds it.

#### How to fix it — step by step

1. **Add the field to `CreateTransferBody`** in `src/domain/transfers/routes.rs`:
   ```rust
   #[derive(Deserialize)]
   #[serde(rename_all = "camelCase")]
   struct CreateTransferBody {
       #[serde(default)]
       idempotency_key: Option<String>,
       quote_id: String,
       recipient_id: String,
       #[serde(default)]
       trade_description: Option<String>,
   }
   ```

2. **Add the field to `CreateTransferInput`** in `src/domain/transfers/service.rs`:
   ```rust
   pub struct CreateTransferInput {
       pub idempotency_key: String,
       pub quote_id: String,
       pub recipient_id: String,
       pub trade_description: Option<String>,
   }
   ```

3. **Wire it through** in the route handler's call to `service::create_transfer`:
   ```rust
   CreateTransferInput {
       idempotency_key,
       quote_id: body.quote_id,
       recipient_id: body.recipient_id,
       trade_description: body.trade_description,
   }
   ```

4. **Wire it into `InsertTransfer`** in `service.rs`:
   ```rust
   InsertTransfer {
       ...
       trade_description: input.trade_description.as_deref(),
       ...
   }
   ```

5. **Add a validation rule**: trim the value and reject if it exceeds 500 characters, to avoid unbounded text storage.

6. **Add a test** in `tests/transfers.rs` that creates a transfer with `tradeDescription` in the body and asserts the returned transfer has the same value.

#### Acceptance criteria

- [ ] `POST /transfers` with `{ "tradeDescription": "Cashew export Q3" }` stores and returns the value.
- [ ] A transfer created without `tradeDescription` still succeeds and returns `null` for the field.
- [ ] `tradeDescription` longer than 500 characters returns `400 VALIDATION`.

---

### ISSUE-04: Onboarding `submit` permits re-submission from `approved` and `rejected` statuses

**Priority:** P1
**Label:** bug
**Files:** `src/domain/onboarding/service.rs`

#### What the issue is

In `service::submit`, the guard that prevents duplicate submissions only checks for `submitted` and `in_review`:

```rust
if app.status == "submitted" || app.status == "in_review" {
    return Err(ApiError::conflict("This application is already being reviewed."));
}
```

A customer whose application was `approved` can call `POST /onboarding/application/submit` again, which will:
- Overwrite `approved_summary` with a freshly generated one (different `account_id`).
- Replace the KYB check records.
- Reset `submitted_at` and `reviewed_at`.

Similarly, a `rejected` application can be re-submitted silently, which may be intentional for appeals — but the current code does not enforce any re-submission rules (e.g. requiring the customer to correct the rejected fields first). The missing guard for `approved` is an unambiguous bug; the `rejected` path needs an explicit business decision and comment.

#### How to fix it — step by step

1. **Block re-submission from `approved`** unconditionally:
   ```rust
   if app.status == "approved" {
       return Err(ApiError::conflict(
           "Your application has already been approved.",
       ));
   }
   ```

2. **Decide on the `rejected` re-submission policy.** Options:
   - Allow re-submission from `rejected` (appeals flow) — add a comment explaining this is intentional.
   - Block it and require an ops unlock — add the same conflict guard as `approved`.
   - Allow it but only after the customer has updated at least one field — check `updated_at > reviewed_at` before permitting.

3. **Update the status guard comment** to explicitly list all statuses and their intended behavior.

4. **Add tests** in `tests/onboarding.rs` for the approved-then-submit path (should get 409) and the rejected-then-submit path (should behave per chosen policy).

#### Acceptance criteria

- [ ] Calling `POST /onboarding/application/submit` on an `approved` application returns `409 CONFLICT`.
- [ ] The chosen policy for `rejected` re-submission is documented in code and covered by a test.
- [ ] The existing 49 passing tests continue to pass.

---

### ISSUE-05: KYB `submit` blocks the axum worker thread during simulated delay

**Priority:** P1
**Label:** bug
**Files:** `src/domain/onboarding/service.rs`, `src/domain/onboarding/kyb.rs`

#### What the issue is

`service::submit` calls `kyb::run_checks(&app, state.config.kyb_check_delay_ms).await` synchronously within the HTTP handler. `kyb::run_checks` sleeps for `kyb_check_delay_ms` milliseconds per check (5 checks × 600 ms default = 3 seconds). This holds the axum worker task for 3 seconds, blocking the thread from handling other requests during that time. Under load, this causes request queuing and latency spikes on unrelated routes.

The comment at the top of `kyb.rs` says "Swap this module for a real provider" — a real provider would make an outbound HTTP call (async I/O, non-blocking) rather than `tokio::time::sleep` inside the handler. But even with a real provider, the handler should not wait for the full KYB result synchronously if the intent is to show a progress UI.

#### How to fix it — step by step

1. **Short-term fix (non-breaking):** Move the KYB execution to a `tokio::spawn` background task. Change `service::submit` to:
   - Transition to `in_review` synchronously and return the application in `in_review` status immediately (HTTP 200 or 202).
   - Spawn a task that runs `kyb::run_checks`, then transitions to `approved` or `rejected` and updates the DB.
   - The frontend polls `GET /onboarding/application` to observe the terminal status.

2. **Update the routes handler** to return the application immediately after committing `in_review`, not after KYB resolves.

3. **Add a polling test**: assert that after calling submit, the application is in `in_review`, then drive the KYB result by waiting (or injecting a test hook), then assert the final status is `approved` or `rejected`.

4. **Long-term:** When a real KYB provider is wired, add a webhook receiver (`POST /onboarding/kyb/webhook`) that receives the provider's async callback and applies the `approved`/`rejected` transition. Remove the polling model.

#### Acceptance criteria

- [ ] `POST /onboarding/application/submit` returns within 100 ms regardless of `KYB_CHECK_DELAY_MS`.
- [ ] The application is in `in_review` immediately after submit returns.
- [ ] The application transitions to `approved` or `rejected` asynchronously.
- [ ] Existing KYB trigger tests (reject-by-name, reject-by-BVN, reject-by-sanctions) still pass.

---

### ISSUE-06: `retry_document_upload` marks any document as `uploaded` regardless of its current status

**Priority:** P1
**Label:** bug
**Files:** `src/domain/onboarding/service.rs`, `src/domain/onboarding/repo.rs`

#### What the issue is

`service::retry_document_upload` fetches the document, verifies ownership, then calls `repo::mark_document_uploaded` unconditionally. It does not check whether the document's `status` is `failed` before proceeding. This means a customer can call `POST /onboarding/application/documents/{id}/retry` on a document that is already `uploaded` (or `uploading`) and silently reset its `uploaded_at` timestamp to `now()`, which corrupts audit timelines and could confuse the frontend's document state machine.

#### How to fix it — step by step

1. **Add a status guard** in `service::retry_document_upload`, after `find_document` returns:
   ```rust
   if found.document.status != "failed" {
       return Err(ApiError::conflict(
           "Only documents in a failed state can be retried.",
       ));
   }
   ```

2. **Verify the `FoundDocument` struct exposes `status`** — it does via `found.document.status`.

3. **Add a test** in `tests/onboarding.rs` that uploads a document successfully, then calls the retry endpoint, and asserts it gets `409 CONFLICT`.

4. **Add a test** that calls retry on a document whose status has been manually set to `failed` in the DB and asserts `200 OK`.

#### Acceptance criteria

- [ ] `POST /onboarding/application/documents/{id}/retry` on an `uploaded` document returns `409 CONFLICT`.
- [ ] `POST /onboarding/application/documents/{id}/retry` on a `failed` document returns `200 OK` and transitions the document to `uploaded`.

---

### ISSUE-07: `tagged_reference` uses `rand::thread_rng()` — collisions possible under load

**Priority:** P1
**Label:** bug
**Files:** `src/util.rs`, `src/domain/transfers/engine.rs`

#### What the issue is

`tagged_reference` in `src/util.rs` generates transfer references (`KM-XXXXXX`), funding references (`FR-XXXXXX`), and payout references (`PO-XXXXXX`) using a 6-character Crockford alphabet string:

```rust
let mut rng = rand::thread_rng();
let body: String = (0..6)
    .map(|_| REF_ALPHABET[rng.gen_range(0..REF_ALPHABET.len())] as char)
    .collect();
```

The alphabet has 32 characters. A 6-character string gives 32^6 = 1,073,741,824 possible values. While this is large, `transfers.reference` has a `unique` constraint, so a collision causes the `INSERT` to fail. There is no retry logic — the service returns a `500 SERVER_ERROR` to the client. Under sustained load with many concurrent transfers this becomes a live incident risk.

Additionally, `thread_rng` is not seeded deterministically in tests, so tests that assert on reference format are not reproducible.

#### How to fix it — step by step

1. **Increase reference entropy**: Extend the body to 8 characters (32^8 = 1,099,511,627,776 combinations) at minimal cost.

2. **Add a retry loop** in `service::create_transfer` around the `repo::insert` call: if the insert returns a unique violation specifically on the `reference` column (not the idempotency key), regenerate the reference and retry up to 3 times before returning `SERVER_ERROR`.

3. **Consider a counter-based scheme** for production: prefix the random body with an encoded timestamp fragment to make collisions structurally impossible (`KM-<base32_millis>-<2_random_chars>`).

4. **In tests**, inject a deterministic `reference` via `InsertTransfer` directly rather than relying on `tagged_reference`, so test references are stable and reproducible.

#### Acceptance criteria

- [ ] `tagged_reference` generates 8-character bodies.
- [ ] A reference unique-violation during insert is caught and retried up to 3 times.
- [ ] Tests that create transfers assert a consistent reference format without being brittle.

---

### ISSUE-08: Quote `rate` stored as `DOUBLE PRECISION` — floating-point rounding on large amounts

**Priority:** P1
**Label:** bug
**Files:** `migrations/004_quotes.sql`, `src/domain/quote.rs`, `src/domain/transfers/engine.rs`

#### What the issue is

The `quotes` table stores `rate` as `double precision` (PostgreSQL's 64-bit float). Quote arithmetic in `src/domain/quote.rs` uses `f64` throughout:

```rust
let (send_minor, receive_minor) =
    derive_amounts(body.amount_field, body.amount.amount_minor, indicative.rate);
```

For large B2B transfers (e.g., USD 78,000 at rate 1645.2), the floating-point multiplication introduces sub-cent errors. Example: `7_800_000 * 1645.2 = 12_832_560_000.000002` in f64. Over thousands of transactions these errors accumulate in the ledger and break reconciliation. The correct type for financial rates is `NUMERIC` in Postgres and `rust_decimal::Decimal` in Rust.

#### How to fix it — step by step

1. **Add `rust_decimal` and `rust_decimal_macros`** to `Cargo.toml`, with the `sqlx` feature enabled:
   ```toml
   rust_decimal = { version = "1", features = ["serde-with-str"] }
   ```

2. **Create migration `006_rate_numeric.sql`** (or include in a larger migration):
   ```sql
   alter table quotes
     alter column rate type numeric(20,8) using rate::numeric(20,8);
   alter table fx_rates
     alter column rate type numeric(20,8) using rate::numeric(20,8);
   ```

3. **Update `src/domain/quote.rs`**: change `rate: f64` in `QuoteRow` to `rate: Decimal`, update `derive_amounts` to use `Decimal` arithmetic, and convert to minor units with `(amount * rate).round().to_i64()`.

4. **Update `src/domain/fx.rs`**: change `FxRow.rate` to `Decimal` and remove the `f64` jitter arithmetic (or keep it but convert to/from `Decimal` cleanly).

5. **Update `src/contract/quote.rs`**: change the `rate` field in `CostBreakdown` and `FirmQuote` to serialize as a string or number. Use `serde_with` to serialize `Decimal` as a JSON number.

6. **Run the existing 49 tests** and fix any type mismatch compile errors.

#### Acceptance criteria

- [ ] `quotes.rate` and `fx_rates.rate` are `NUMERIC(20,8)` in the schema.
- [ ] `derive_amounts` uses `Decimal` multiplication with no `f64` intermediate.
- [ ] A transfer for USD 78,000 at rate 1645.2 produces exactly `128,325,600` minor units for NGN (i.e., NGN 1,283,256.00), with no floating-point rounding error.
- [ ] All 49 existing tests pass.

---

### ISSUE-09: Spawned `transfer_auto_advance` tasks are never cancelled on server shutdown

**Priority:** P1
**Label:** bug
**Files:** `src/domain/transfers/service.rs`, `src/main.rs`

#### What the issue is

`schedule_simulated_progression` in `service.rs` calls `tokio::spawn(async move { ... })` to drive the transfer state machine forward after a delay. These tasks hold a clone of `AppState` (which includes the `PgPool`). When the server shuts down (e.g., `Ctrl+C`), `axum::serve` resolves but these background tasks continue running, attempting to write to the database against a pool that may be mid-teardown. In production Kubernetes environments this causes spurious `SQLSTATE 57P01` (admin shutdown) errors logged as warnings, and in tests it can cause tasks spawned by one test to bleed into the next.

#### How to fix it — step by step

1. **Add a shutdown signal channel** to `AppState`:
   ```rust
   pub struct AppState {
       pub pool: PgPool,
       pub config: Arc<Config>,
       pub shutdown: tokio::sync::watch::Receiver<bool>,
   }
   ```
   In `main.rs`, create `let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false)` and pass `shutdown_rx` into `AppState`.

2. **Update `schedule_simulated_progression`** to select on the shutdown signal:
   ```rust
   tokio::spawn(async move {
       tokio::select! {
           _ = tokio::time::sleep(Duration::from_millis(auto_ms as u64)) => {
               // proceed with advance_to_completion
           }
           _ = state.shutdown.changed() => {
               tracing::debug!(%transfer_id, "simulated progression cancelled (shutdown)");
               return;
           }
       }
       // ... advance_to_completion call
   });
   ```

3. **In `main.rs`**, listen for `SIGTERM`/`SIGINT` and send `shutdown_tx.send(true)` before the process exits:
   ```rust
   tokio::signal::ctrl_c().await?;
   shutdown_tx.send(true).ok();
   ```

4. **In tests**, use `Config::test()` which sets `transfer_auto_advance_ms = -1`, so no background tasks are spawned during test runs (the existing guard already handles this).

#### Acceptance criteria

- [ ] Sending `SIGINT` to the server while a transfer is in progress does not produce any database error logs.
- [ ] Integration tests are not affected (auto-advance is disabled in test config).
- [ ] The `AppState` shutdown receiver is cloned cleanly alongside the pool.

---

### ISSUE-10: `src/contract` is a hand-maintained mirror of frontend TypeScript types with no codegen

**Priority:** P1
**Label:** tech-debt
**Files:** `src/contract/` (all files), `Kimana_frontend/src/api/` (type definitions)

#### What the issue is

All structs under `src/contract/` (e.g., `transfer.rs`, `onboarding.rs`, `dashboard.rs`, `quote.rs`, `common.rs`) are manually written Rust mirrors of the TypeScript types defined in the frontend. There is no automated synchronization. Every time the frontend adds or renames a field (e.g., a new field on `BusinessDetails`, a new `TransferStatus` variant), the Rust contract must be updated by hand. Missed updates cause silent JSON deserialization failures or missing fields in API responses.

Concrete drift already present: `src/contract/auth.rs` has a `SessionResponse` with `user_id: String` but the frontend's session type may include additional fields; `src/contract/onboarding.rs::BusinessDetails` may be missing the `email` / `password` fields that the frontend sends during onboarding.

#### How to fix it — step by step

1. **Audit the existing drift** by comparing each `src/contract/*.rs` struct field-by-field against the corresponding frontend type in `Kimana_frontend/src/`. Document every discrepancy.

2. **Fix immediate drift** (especially `BusinessDetails` — see ISSUE-11) manually as a short-term patch.

3. **Evaluate codegen options:**
   - **Option A — `typeshare`**: Annotate Rust structs with `#[typeshare]` and run `typeshare-cli` as a `build.rs` or CI step to generate a `types.ts` file in the frontend. The frontend consumes the generated file as its source of truth.
   - **Option B — JSON Schema**: Generate JSON Schema from Rust types using `schemars`, and generate TypeScript types from the schema using `json-schema-to-typescript`.
   - **Option C — OpenAPI**: Use `utoipa` to annotate handlers and generate an OpenAPI spec, then generate TypeScript types from the spec.

4. **Implement the chosen approach**: Add the codegen step to `Makefile` or a `scripts/` entry. Run it in CI on every PR that touches `src/contract/`. Fail CI if the generated output is not committed.

5. **Delete the manually written frontend type definitions** that are now generated, updating all imports.

#### Acceptance criteria

- [ ] All `src/contract/` structs exactly match their frontend counterparts — no silent field drift.
- [ ] A `make generate-types` (or equivalent) command regenerates the TypeScript types from the Rust source.
- [ ] CI fails if `src/contract/` is modified without regenerating the frontend types.

---

### ISSUE-11: `BusinessDetails` contract struct is missing `email` and `password` fields

**Priority:** P1
**Label:** bug
**Files:** `src/contract/onboarding.rs`

#### What the issue is

The frontend's onboarding form sends a `BusinessDetails` object that includes `email` and potentially `password` fields for account setup. The `BusinessDetails` struct in `src/contract/onboarding.rs` does not include these fields. Serde silently drops unknown fields by default (`#[serde(deny_unknown_fields)]` is not set), so the fields are received and discarded without error. The customer's email is never stored, meaning there is no email on record for any business that completes onboarding — breaking any future notification, login, or verification flow.

#### How to fix it — step by step

1. **Inspect the frontend's `BusinessDetails` form** in `Kimana_frontend/src/features/onboarding/` to identify all fields the form submits.

2. **Add the missing fields** to `src/contract/onboarding.rs`:
   ```rust
   pub struct BusinessDetails {
       // ... existing fields ...
       pub email: Option<String>,
       // Do NOT store raw passwords — see step 3
   }
   ```

3. **Do not store `password` in `business_details` JSONB.** If the onboarding flow doubles as account creation, extract the password from the request body separately in the handler, hash it with bcrypt, and store it on the `users` row. Never store a plaintext or hashed password inside the `onboarding_applications.business` JSONB column.

4. **Add email validation** in `src/domain/onboarding/schema.rs`:
   - Email must be present and match a basic format (use the `email_address` crate or a simple regex).
   - Email must be unique across `users` (add a unique index on a new `email` column or on the JSONB extraction).

5. **Add a migration** to add `email text unique` to the `users` table and populate it from existing seed data.

#### Acceptance criteria

- [ ] `PUT /onboarding/application/business` stores the customer's email.
- [ ] Submitting an invalid or duplicate email returns `400 VALIDATION`.
- [ ] The `password` field, if sent, is never stored in plaintext anywhere in the database.
- [ ] `GET /onboarding/application` returns the stored email as part of `businessDetails`.

---

### ISSUE-12: No integration test covers the rejected KYB path end-to-end

**Priority:** P1
**Label:** tech-debt
**Files:** `tests/onboarding.rs`, `src/domain/onboarding/kyb.rs`

#### What the issue is

`src/domain/onboarding/kyb.rs` documents three rejection triggers:
- Legal name containing `"reject"` fails `cac_lookup`.
- A principal BVN of `"00000000000"` fails `director_identity`.
- A principal name containing `"sanction"` fails `sanctions_pep`.

`tests/onboarding.rs` tests the happy path extensively (draft → submit → approved) but there is no test that exercises any of the three rejection triggers end-to-end (submit → rejected, verify `rejectionReasons`, verify status). If someone refactors `kyb.rs` and accidentally breaks the rejection path, no test will catch it.

#### How to fix it — step by step

1. **Add a test `submit_rejected_by_legal_name`** in `tests/onboarding.rs`:
   - Save business details with `legal_name: "Reject Me Ltd"`.
   - Submit the application.
   - Assert the returned status is `"rejected"`.
   - Assert `rejectionReasons` contains an entry for `"business.cacNumber"`.

2. **Add a test `submit_rejected_by_bvn`**:
   - Save a principal with `bvn: "00000000000"`.
   - Submit.
   - Assert `"rejected"` and `rejectionReasons` for `"principals[].bvn"`.

3. **Add a test `submit_rejected_by_sanctions`**:
   - Save a principal with `full_name: "Mr Sanction Test"`.
   - Submit.
   - Assert `"rejected"` and `rejectionReasons` for `"principals[].fullName"`.

4. **Ensure the tests reseed** between runs using `app.reseed()` to avoid state leakage.

#### Acceptance criteria

- [ ] Three new rejection-path integration tests exist and pass.
- [ ] Each test asserts the specific `rejectionReasons` field from the API response.
- [ ] Tests are isolated and pass in any order.

---

### ISSUE-13: Tests serialised via file lock — no parallel test execution, slow CI

**Priority:** P1
**Label:** tech-debt
**Files:** `tests/common/mod.rs`, all test files

#### What the issue is

`TestApp::new()` calls `seed::seed(&pool).await` which does a `TRUNCATE ... CASCADE` on all application tables and rebuilds them. Because tests share a single database and all start with a full truncate+reseed, they must run serially — `cargo test` uses the `--test-threads=1` flag (or a file-lock mutex, depending on the setup). This makes the test suite slow as the number of tests grows. The current 49 tests are manageable; at 150+ tests this becomes a real CI bottleneck.

#### How to fix it — step by step

1. **Give each test its own database schema.** In `TestApp::new()`, create a unique schema per test run using a random UUID:
   ```rust
   let schema = format!("test_{}", Uuid::new_v4().to_string().replace('-', ""));
   sqlx::query(&format!("CREATE SCHEMA {schema}")).execute(&pool).await?;
   sqlx::query(&format!("SET search_path TO {schema}")).execute(&pool).await?;
   ```
   Run migrations and seed against that schema, then drop it in a `Drop` impl on `TestApp`.

2. **Alternative (simpler): use `sqlx::test`** macro, which spins up a fresh database per test function using SQLx's built-in test isolation. Refactor `TestApp` to wrap `sqlx::PgPool` provided by `#[sqlx::test(migrations = "./migrations")]`.

3. **Remove the serial constraint**: with schema isolation or per-test databases, tests can run with `--test-threads` at the default (number of CPUs).

4. **Update `tests/common/mod.rs`** accordingly and remove any explicit `--test-threads=1` flags from `Makefile` or `.cargo/config.toml`.

#### Acceptance criteria

- [ ] `cargo test` with default thread count (no `--test-threads=1`) passes without race conditions.
- [ ] All 49 existing tests still pass.
- [ ] CI test time is measurably reduced (target: under 30 seconds for the full suite).

---

### ISSUE-14: `TestApp::send` never sets `Idempotency-Key` header — transfer tests bypass header validation

**Priority:** P1
**Label:** tech-debt
**Files:** `tests/common/mod.rs`, `tests/transfers.rs`

#### What the issue is

The `TestApp::send` helper in `tests/common/mod.rs` builds requests without an `Idempotency-Key` header. The transfer creation handler in `src/domain/transfers/routes.rs` reads the key from the header first, then falls back to `body.idempotency_key`:

```rust
let idempotency_key = headers
    .get("idempotency-key")
    .and_then(|v| v.to_str().ok())
    .map(String::from)
    .or(body.idempotency_key)
    .unwrap_or_default();
```

All transfer creation tests pass `idempotencyKey` in the JSON body, never in the header. This means the header path is completely untested. If the header parsing is broken, no test will catch it. It also means the body-fallback path (which the real frontend may not use) is the only tested path.

#### How to fix it — step by step

1. **Add a `post_with_headers` method** to `TestApp`:
   ```rust
   pub async fn post_with_headers(
       &self,
       uri: &str,
       body: Value,
       headers: &[(&str, &str)],
   ) -> (StatusCode, Value) { ... }
   ```

2. **Add a test** in `tests/transfers.rs` that creates a transfer using `post_with_headers` with `Idempotency-Key` set in the header (not the body) and asserts success.

3. **Add a test** that sends `Idempotency-Key` in both the header and the body with different values, and asserts that the header value takes precedence.

4. **Add a test** that sends no idempotency key at all (neither header nor body) and asserts `400 VALIDATION`.

#### Acceptance criteria

- [ ] A test exercises the `Idempotency-Key` header path for transfer creation.
- [ ] A test confirms header takes precedence over body field.
- [ ] Missing idempotency key returns `400`.

---

### ISSUE-15: `idempotency_key` validation only checks length, not format

**Priority:** P1
**Label:** bug
**Files:** `src/domain/transfers/service.rs`

#### What the issue is

```rust
if input.idempotency_key.trim().len() < 8 {
    return Err(ApiError::validation("Provide a stable idempotency key."));
}
```

The only validation is `len() >= 8`. A key of 8 spaces passes. A key containing SQL injection characters, emoji, or null bytes passes. The key is stored in the `transfers` table and returned in API responses, so arbitrary Unicode or control characters can corrupt client-side parsing and logging.

#### How to fix it — step by step

1. **Define an allowed character set**: alphanumeric, hyphens, and underscores. Maximum 128 characters.

2. **Update the validation in `service::create_transfer`**:
   ```rust
   let key = input.idempotency_key.trim();
   if key.len() < 8 || key.len() > 128 {
       return Err(ApiError::validation("Idempotency key must be 8–128 characters."));
   }
   if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
       return Err(ApiError::validation(
           "Idempotency key may only contain letters, digits, hyphens, and underscores.",
       ));
   }
   ```

3. **Add tests** for: all-spaces key (rejected), key with emoji (rejected), key with SQL characters (rejected), valid key of exactly 8 characters (accepted), valid key of 128 characters (accepted), key of 129 characters (rejected).

#### Acceptance criteria

- [ ] Keys shorter than 8 characters return `400`.
- [ ] Keys longer than 128 characters return `400`.
- [ ] Keys containing non-alphanumeric characters other than `-` and `_` return `400`.
- [ ] Valid keys (8-128 alphanumeric/hyphen/underscore) are accepted.

---

## P2 — Transfer Lifecycle Gaps

---

### ISSUE-16: `AWAITING_FUNDS` transfers have no expiry timer — a parked transfer stays parked forever

**Priority:** P2
**Label:** bug
**Files:** `src/domain/transfers/service.rs`, `src/domain/transfers/engine.rs`

#### What the issue is

Once a transfer reaches `AWAITING_FUNDS`, it waits for the customer to send funds. There is no timeout. If a customer creates a transfer, receives a funding reference, and never sends the money, the transfer stays in `AWAITING_FUNDS` permanently. This:
- Pollutes the "in-progress" count on the dashboard indefinitely.
- Holds the quote snapshot with a locked rate that no longer applies.
- Prevents proper reporting of expired/abandoned transfers.

The `expire_transfer` engine function exists (`engine::expire_transfer`) but is never called on a timer.

#### How to fix it — step by step

1. **Add a config variable** `AWAITING_FUNDS_TIMEOUT_SECONDS` (default: 86400 — 24 hours) to `Config`.

2. **In `service::create_transfer`**, after calling `engine::advance_to_awaiting_funds`, spawn a delayed task that calls `engine::expire_transfer` if the transfer is still in `AWAITING_FUNDS` after the timeout:
   ```rust
   if state.config.transfer_auto_advance_ms >= 0 {
       let state2 = state.clone();
       let timeout = state.config.awaiting_funds_timeout_seconds;
       tokio::spawn(async move {
           tokio::time::sleep(Duration::from_secs(timeout)).await;
           // only expire if still in AWAITING_FUNDS
           if let Ok(Some(o)) = repo::get_owner_and_status(&state2.pool, &transfer_id.to_string()).await {
               if o.current_status == TransferStatus::AwaitingFunds {
                   let _ = engine::expire_transfer(&state2, transfer_id, None).await;
               }
           }
       });
   }
   ```

3. **Also apply the shutdown signal** (from ISSUE-09) to this expiry task so it cancels cleanly.

4. **Add an integration test** that creates a transfer with a very short timeout (set via config), waits for it to expire, and asserts the status is `EXPIRED`.

#### Acceptance criteria

- [ ] A transfer still in `AWAITING_FUNDS` after the configured timeout automatically transitions to `EXPIRED`.
- [ ] A transfer that receives funds before the timeout is NOT expired.
- [ ] `AWAITING_FUNDS_TIMEOUT_SECONDS` is documented in `.env.example`.

---

### ISSUE-17: After server restart, transfers parked at `AWAITING_FUNDS` are never resumed

**Priority:** P2
**Label:** bug
**Files:** `src/main.rs`, `src/domain/transfers/service.rs`

#### What the issue is

The simulated progression and the `AWAITING_FUNDS` expiry timer (ISSUE-16) are in-memory `tokio::spawn` tasks. If the server restarts while a transfer is parked at `AWAITING_FUNDS`, those tasks are gone. The transfer sits in the database indefinitely with no timer to advance or expire it. The comment in `service.rs` acknowledges this: `"A restart between AWAITING_FUNDS and COMPLETED leaves the transfer parked — no resume sweep."` This is a data consistency issue: the system's in-memory state and database state diverge permanently after a restart.

#### How to fix it — step by step

1. **Add a startup sweep in `main.rs`**, after `db::run_migrations` and before `axum::serve`:
   ```rust
   transfers::service::resume_parked_transfers(&state).await?;
   ```

2. **Implement `resume_parked_transfers`** in `service.rs`:
   - Query all transfers with `current_status = 'AWAITING_FUNDS'`.
   - For each, calculate how long it has been in that state using `state_entered_at` from `transfer_state_history`.
   - If already past the expiry timeout, call `engine::expire_transfer` immediately.
   - If not yet past the timeout, spawn a delayed `expire_transfer` task for the remaining time.

3. **Query `state_entered_at`** from the `transfer_state_history` table: find the most recent row for each transfer where `status = 'AWAITING_FUNDS'` and use its `entered_at`.

4. **Add an integration test**: seed a transfer directly in `AWAITING_FUNDS` with `entered_at` set to 25 hours ago, call `resume_parked_transfers`, and assert it transitions to `EXPIRED`.

#### Acceptance criteria

- [ ] On startup, all transfers stuck in `AWAITING_FUNDS` past their timeout are immediately expired.
- [ ] Transfers in `AWAITING_FUNDS` within their timeout window get a correctly scheduled expiry task.
- [ ] The sweep completes before the server starts accepting requests.

---

### ISSUE-18: `GET /transfers` has no pagination — will full-scan as transfer volume grows

**Priority:** P2
**Label:** enhancement
**Files:** `src/domain/transfers/routes.rs`, `src/domain/transfers/repo.rs`

#### What the issue is

`repo::list_by_customer` issues an unbounded `SELECT * FROM transfers WHERE customer_id = $1 ORDER BY created_at DESC`. For a business with thousands of transfers, this returns the entire history in a single query and serializes it all into one JSON response, consuming significant memory and network bandwidth. The frontend's transfers table will hang or show stale data as volume grows.

#### How to fix it — step by step

1. **Add pagination parameters** to the list route: `?page=1&pageSize=20` (offset/limit) or `?before=<created_at_cursor>` (cursor-based). Cursor-based is preferred for consistency with sorted results.

2. **Update `ListQuery`** in `routes.rs`:
   ```rust
   #[derive(Deserialize)]
   struct ListQuery {
       status: Option<String>,
       before: Option<String>,   // ISO-8601 cursor
       limit: Option<i64>,
   }
   ```

3. **Update `repo::list_by_customer`** to accept `before: Option<DateTime<Utc>>` and `limit: i64`:
   ```sql
   WHERE customer_id = $1
     AND ($2::timestamptz IS NULL OR created_at < $2)
   ORDER BY created_at DESC
   LIMIT $3
   ```

4. **Return a pagination envelope** from the route:
   ```json
   { "items": [...], "nextCursor": "2026-08-20T09:00:00.000Z", "hasMore": true }
   ```

5. **Update the frontend client** (`Kimana_backend/integration/live-api-client.ts`) to handle the paginated response.

6. **Add a test** that creates 25 transfers and asserts that `GET /transfers?limit=20` returns 20 items with `hasMore: true`, and the second page returns the remaining 5 with `hasMore: false`.

#### Acceptance criteria

- [ ] `GET /transfers` with no pagination params returns the first 20 transfers (sensible default).
- [ ] `GET /transfers?limit=50` returns up to 50.
- [ ] `GET /transfers?before=<cursor>` returns transfers older than the cursor.
- [ ] Response includes `hasMore` and `nextCursor` fields.

---

### ISSUE-19: `listTransfers` only supports `?status=` filter — missing date range, reference, and currency filters

**Priority:** P2
**Label:** enhancement
**Files:** `src/domain/transfers/routes.rs`, `src/domain/transfers/repo.rs`

#### What the issue is

`GET /transfers` accepts only a single optional `?status=` query parameter. The frontend dashboard has a transfers table that will need to support: date range filtering (show transfers for a given month), currency filtering (show only USD transfers), and reference search (find a specific `KM-XXXXXX` reference). Without these filters, the frontend has to download the entire transfer list and filter client-side, which breaks entirely once pagination (ISSUE-18) is added.

#### How to fix it — step by step

1. **Extend `ListQuery`** to include:
   ```rust
   struct ListQuery {
       status: Option<String>,
       currency: Option<String>,
       reference: Option<String>,      // partial match on reference
       from: Option<String>,           // ISO-8601 date
       to: Option<String>,             // ISO-8601 date
       before: Option<String>,         // pagination cursor
       limit: Option<i64>,
   }
   ```

2. **Build the SQL query dynamically** in `repo.rs` using SQLx's query builder or a hand-built parameterized WHERE clause with conditional fragments.

3. **Validate inputs**: parse `currency` as `CurrencyCode`, parse `from`/`to` as `DateTime<Utc>`, sanitize `reference` to alphanumeric/hyphen only (to prevent injection).

4. **Add tests** for each filter in isolation and in combination.

#### Acceptance criteria

- [ ] `GET /transfers?currency=USD` returns only USD transfers.
- [ ] `GET /transfers?from=2026-08-01&to=2026-08-31` returns transfers within that window.
- [ ] `GET /transfers?reference=KM-2H4F` performs a prefix/contains match and returns matching transfers.
- [ ] Invalid `currency` values return `400 VALIDATION`.

---

### ISSUE-20: Transfer `GET /transfers/{id}/timeline` exposes ops-only notes to customers

**Priority:** P2
**Label:** bug
**Files:** `src/domain/transfers/service.rs`, `src/domain/transfers/repo.rs`

#### What the issue is

`repo::get_history` fetches all `transfer_state_history` rows including the `note` column. The `note` field is intended for ops-only annotations (e.g., "Flagged for manual review — suspicious beneficiary"). The service currently returns notes to any authenticated customer via `GET /transfers/{id}/timeline`. There is no role check. Any note an operator adds to the timeline will be visible to the customer.

#### How to fix it — step by step

1. **Add a `note_visibility` column** to `transfer_state_history`:
   ```sql
   alter table transfer_state_history
     add column note_visibility text not null default 'ops'
     check (note_visibility in ('ops', 'customer'));
   ```

2. **In `repo::get_history`**, accept a `role: &str` parameter and filter note visibility:
   ```rust
   pub async fn get_history(pool, transfer_id, role: &str) -> ApiResult<Vec<...>> {
       // if role != "operator", set note to None for rows where note_visibility = 'ops'
   }
   ```

3. **Update `service::get_timeline`** to pass `session.role_str()` to `repo::get_history`.

4. **Add a test** that inserts an ops-only note directly in the DB, calls the timeline as a customer, and asserts the note is `null` in the response.

#### Acceptance criteria

- [ ] Notes with `note_visibility = 'ops'` are not returned to `role = 'customer'` callers.
- [ ] Notes with `note_visibility = 'customer'` are returned to all callers.
- [ ] Existing tests that assert timeline content still pass.

---

### ISSUE-21: `AccountBalance.pending` is always `None` — in-transit debits not reflected

**Priority:** P2
**Label:** bug
**Files:** `src/domain/ledger.rs`, `src/contract/ledger.rs`

#### What the issue is

`get_balances` in `src/domain/ledger.rs` returns `pending: None` for every account balance. The `AccountBalance` contract type has a `pending: Option<Money>` field specifically to show funds that are committed (debited from the account) but not yet completed (still in `AWAITING_FUNDS`, `FUNDED`, `SETTLING`, or `PAYING_OUT`). A customer looking at their USD balance cannot tell that $18,500 is tied up in an in-progress transfer. This can lead to double-counting and customer confusion.

#### How to fix it — step by step

1. **Calculate pending balance** as the sum of `send_amount_minor` for all transfers belonging to this customer in non-terminal, post-`CREATED` states:
   ```sql
   SELECT coalesce(sum(t.send_amount_minor), 0)::bigint
   FROM transfers t
   WHERE t.customer_id = $1
     AND t.send_currency = $2
     AND t.current_status IN ('AWAITING_FUNDS', 'FUNDED', 'SETTLING', 'SETTLED', 'PAYING_OUT')
   ```

2. **Update `get_balances`** to run this query per currency and populate `pending`:
   ```rust
   AccountBalance {
       ...,
       pending: if pending_minor > 0 {
           Some(Money::new(pending_minor, currency))
       } else {
           None
       },
   }
   ```

3. **Add a test** in `tests/dashboard.rs` or a new `tests/ledger.rs`: create a transfer, check that `pending` reflects the `send_amount_minor`, complete the transfer, check that `pending` drops to zero.

#### Acceptance criteria

- [ ] While a transfer is in `AWAITING_FUNDS` through `PAYING_OUT`, the send-currency account shows a non-null `pending` equal to the `sendAmount` of all such transfers.
- [ ] Once a transfer reaches `COMPLETED`, `REJECTED`, `EXPIRED`, or `REVERSED`, the `pending` amount decreases accordingly.
- [ ] `pending` is `null` (not `{"amountMinor":0}`) when there are no in-transit transfers.

---

### ISSUE-22: `GET /ledger/statement` endpoint is not implemented

**Priority:** P2
**Label:** enhancement
**Files:** `src/domain/ledger.rs`, `src/lib.rs`, `Kimana_backend/integration/live-api-client.ts`

#### What the issue is

The `live-api-client.ts` in `integration/` defines `ledger.getStatement()` as a callable function, implying the backend should expose a `GET /ledger/statement` endpoint that returns a paginated list of ledger entries for a customer's account. This endpoint does not exist. The frontend's reconciliation tab and any statement download feature will have no data source.

#### How to fix it — step by step

1. **Design the endpoint**: `GET /ledger/statement?currency=USD&from=2026-08-01&to=2026-08-31&limit=50&before=<cursor>`.

2. **Add a `LedgerEntry` contract struct** in `src/contract/ledger.rs`:
   ```rust
   pub struct LedgerEntry {
       pub id: String,
       pub account_id: String,
       pub transfer_id: Option<String>,
       pub amount_minor: i64,
       pub amount: Money,
       pub running_balance: Money,
       pub description: String,
       pub posted_at: String,
   }
   ```

3. **Implement the query** in `src/domain/ledger.rs`:
   ```sql
   SELECT le.* FROM ledger_entries le
   JOIN accounts a ON a.id = le.account_id
   WHERE a.customer_id = $1
     AND a.currency = $2
     AND ($3::timestamptz IS NULL OR le.posted_at >= $3)
     AND ($4::timestamptz IS NULL OR le.posted_at < $4)
   ORDER BY le.posted_at DESC
   LIMIT $5
   ```

4. **Add the route** in `src/domain/ledger.rs` and register it in `src/lib.rs`.

5. **Add integration tests** for the statement endpoint.

#### Acceptance criteria

- [ ] `GET /ledger/statement?currency=USD` returns a paginated list of ledger entries for the customer's USD account.
- [ ] Each entry includes `id`, `transferId`, `amount`, `runningBalance`, `description`, `postedAt`.
- [ ] Date range filters work correctly.
- [ ] An account with no entries returns an empty list (not 404).

---

### ISSUE-23: `POST /ledger/statement/export` endpoint is not implemented

**Priority:** P2
**Label:** enhancement
**Files:** `src/domain/ledger.rs`

#### What the issue is

Customers need to download their ledger statements as CSV or PDF for accounting and compliance purposes. The `live-api-client.ts` suggests this should be a `POST /ledger/statement/export` endpoint. It does not exist.

#### How to fix it — step by step

1. **Add `POST /ledger/statement/export`** that accepts `{ currency, from, to, format: "csv" | "pdf" }`.

2. **For CSV**: Build the CSV in memory using the `csv` crate, set `Content-Type: text/csv` and `Content-Disposition: attachment; filename="statement-<currency>-<from>-<to>.csv"`.

3. **For PDF**: This requires a PDF generation library (e.g., `printpdf`) or an external service. For the initial implementation, support only CSV and return `400 VALIDATION` with message "Only CSV export is supported." for PDF requests.

4. **Apply the same date range and ownership checks** as `GET /ledger/statement`.

5. **Add a test** that requests a CSV export and checks the `Content-Type` header and that the body starts with the expected CSV header row.

#### Acceptance criteria

- [ ] `POST /ledger/statement/export` with `format: "csv"` returns a downloadable CSV file.
- [ ] The CSV includes columns: `Date`, `Description`, `Amount`, `Currency`, `Running Balance`, `Transfer Reference`.
- [ ] `format: "pdf"` returns `400` with a clear message.
- [ ] Empty date ranges return an empty CSV with only the header row.

---

### ISSUE-24: Dashboard `balance_highlights` returns hardcoded strings referencing seeded transfer refs

**Priority:** P2
**Label:** bug
**Files:** `src/domain/dashboard.rs`

#### What the issue is

`balance_highlights()` in `src/domain/dashboard.rs` is a static function that returns hardcoded `BalanceHighlight` values referencing fake transfer references `TXN-8843` and `TXN-8842`:

```rust
delta_text: "TXN-8843 settling".into(),
delta_text: "TXN-8842 in progress".into(),
```

These strings are not derived from live data. For a real customer with different transfers, the dashboard will show entirely incorrect highlight text. The NGN highlight also shows a hardcoded `"+₦2.4M this month"` delta that is not computed from the ledger.

#### How to fix it — step by step

1. **Replace `balance_highlights()`** with a function that takes `balances: &[AccountBalance]` and `transfers: &[Transfer]` (fetched from the DB).

2. **Derive the delta text** from real data:
   - For each balance, find the most recent in-progress transfer for that currency.
   - If a `SETTLING` or `PAYING_OUT` transfer exists, set `delta_text` to `"{reference} settling"`.
   - If an `AWAITING_FUNDS` or `FUNDED` transfer exists, set `delta_text` to `"{reference} in progress"`.
   - Otherwise, calculate the 30-day volume change from the ledger.

3. **Remove all hardcoded transfer references** from `dashboard.rs`.

4. **Add a test** in `tests/dashboard.rs` that asserts balance highlights reference real seeded transfer data.

#### Acceptance criteria

- [ ] `balance_highlights` field in `GET /dashboard/overview` reflects the real state of the customer's in-progress transfers, not hardcoded strings.
- [ ] No hardcoded transfer reference strings remain in `src/domain/dashboard.rs`.

---

### ISSUE-25: Dashboard `pending_actions` returns hardcoded seeded entries — not derived from live state

**Priority:** P2
**Label:** bug
**Files:** `src/domain/dashboard.rs`

#### What the issue is

`pending_actions()` in `src/domain/dashboard.rs` returns three hardcoded `PendingAction` entries referencing `txn_8842` and `txn_8843`. These transfer IDs are fake (the seeded transfers use UUIDs, not these string IDs). The `transfer_id` fields point to non-existent resources. Any customer who has no documents requiring action will still see these fake pending actions on their dashboard.

#### How to fix it — step by step

1. **Derive pending actions from live data.** Query the database for:
   - Transfers with required but missing trade documents (once ISSUE-39-42 are implemented).
   - Documents in `failed` upload status.
   - Onboarding steps that are incomplete.

2. **For now (before P3 trade documents land)**, return an empty `pending_actions: []` array instead of hardcoded fake entries. An empty array is more honest than fake data.

3. **Remove `pending_actions()`** static function entirely and replace with a real query.

4. **Add a test** that asserts the returned `pendingActions` is an empty array when there are no real pending actions for the seeded customer.

#### Acceptance criteria

- [ ] `pendingActions` in `GET /dashboard/overview` is `[]` when there are no real pending actions.
- [ ] No hardcoded `txn_8842` or `txn_8843` string literals appear in `src/domain/dashboard.rs`.

---

### ISSUE-26: `working_capital_offer` is hardcoded — not derived from customer ledger

**Priority:** P2
**Label:** bug
**Files:** `src/domain/dashboard.rs`

#### What the issue is

```rust
working_capital_offer: Some(WorkingCapitalOffer {
    max_advance: Money::new(3_825_000, CurrencyCode::Usd),
    basis_description: "Against Amsterdam Commodities receivable".into(),
    monthly_rate_percent: 2.5,
}),
```

This hardcoded working capital offer references a specific seeded recipient ("Amsterdam Commodities") and a fixed advance amount that bears no relationship to the customer's actual receivables or transaction history. Every customer sees the same offer regardless of their activity.

#### How to fix it — step by step

1. **Short-term**: Return `working_capital_offer: None` instead of the hardcoded value. This is accurate — no real underwriting logic exists yet.

2. **Long-term (P3/P4)**: Implement a basic eligibility calculation:
   - Query 90-day USD send volume from `transfers`.
   - If volume exceeds a threshold (e.g., $50,000), calculate a max advance as a percentage of that volume.
   - Set `basis_description` dynamically based on the largest recent completed transfer.

3. **Add a test** that asserts `workingCapitalOffer` is `null` before the eligibility logic is implemented.

#### Acceptance criteria

- [ ] `workingCapitalOffer` is `null` in `GET /dashboard/overview` until a real eligibility model is implemented.
- [ ] No hardcoded USD amounts or recipient names remain in `src/domain/dashboard.rs`.

---

### ISSUE-27: `payout_success_rate_percent` and `avg_settlement_seconds` are hardcoded constants

**Priority:** P2
**Label:** bug
**Files:** `src/domain/dashboard.rs`

#### What the issue is

```rust
payout_success_rate_percent: 98.3,
avg_settlement_seconds: 402,
```

These are hardcoded values in `DashboardStats`. They never change and are not computed from any real transfer data. Showing fake performance metrics to a customer is misleading and will erode trust once they notice the numbers never change.

#### How to fix it — step by step

1. **Compute `payout_success_rate_percent`** from real data:
   ```sql
   SELECT
     round(
       100.0 * count(*) filter (where current_status = 'COMPLETED') /
       nullif(count(*) filter (where current_status in ('COMPLETED', 'REJECTED')), 0),
     1) as success_rate
   FROM transfers
   WHERE customer_id = $1
     AND created_at >= now() - interval '90 days'
   ```
   If there are no completed or rejected transfers, return `null` instead of a fake percentage.

2. **Compute `avg_settlement_seconds`** as the average elapsed time between `FUNDED` and `COMPLETED` state transitions by querying `transfer_state_history`:
   ```sql
   SELECT extract(epoch from avg(completed.entered_at - funded.entered_at))
   FROM transfer_state_history funded
   JOIN transfer_state_history completed ON funded.transfer_id = completed.transfer_id
   WHERE funded.status = 'FUNDED' AND completed.status = 'COMPLETED'
     AND funded.transfer_id IN (
       SELECT id FROM transfers WHERE customer_id = $1
     )
   ```

3. **Update `DashboardStats`** to use `Option<f64>` and `Option<i64>` for these fields so they can be `null` when there is no data.

4. **Update the frontend** to handle `null` for these stats gracefully (show `"—"` or `"Not enough data"`).

#### Acceptance criteria

- [ ] `payoutSuccessRatePercent` is computed from real transfer history, not hardcoded.
- [ ] `avgSettlementSeconds` is computed from `transfer_state_history`, not hardcoded.
- [ ] Both fields are `null` when the customer has no completed transfers yet.

---

### ISSUE-28: `usd_send_volume_30d` only counts USD — EUR/GBP corridor volume excluded

**Priority:** P2
**Label:** bug
**Files:** `src/domain/dashboard.rs`

#### What the issue is

```rust
async fn usd_send_volume_30d(pool: &PgPool, customer_id: Uuid) -> ApiResult<i64> {
    let total: i64 = sqlx::query_scalar(
        "select coalesce(sum(send_amount_minor), 0)::bigint from transfers
          where customer_id = $1 and send_currency = 'USD'  -- ← USD only
            and created_at >= now() - interval '30 days'",
    )
```

The 30-day volume metric only counts USD transfers. EUR, GBP, and GHS transfers are excluded. The returned `volume30d: Money` always has `currency: USD`, so a customer who primarily transfers EUR sees `$0` volume even if they have millions in EUR transfers.

#### How to fix it — step by step

1. **Return a per-currency volume breakdown** rather than a single USD total:
   - Query `sum(send_amount_minor)` grouped by `send_currency` for the last 30 days.
   - Return `volume30d: Vec<Money>` in the contract.

2. **Alternatively**, convert all amounts to USD equivalent using the current FX rates and return a single USD total. This is simpler for the frontend but less transparent.

3. **Update `DashboardStats`** contract and the frontend display accordingly.

4. **Update the test** in `tests/dashboard.rs` to assert that EUR transfers are reflected in the volume stat.

#### Acceptance criteria

- [ ] 30-day volume reflects all currencies, not only USD.
- [ ] EUR transfers appear in the volume metric.
- [ ] The frontend can display multi-currency volume (either as a breakdown or a converted total).

---

### ISSUE-29: `bank_name` hardcoded to `"Partner Bank"` on every recipient save

**Priority:** P2
**Label:** bug
**Files:** `src/domain/recipients.rs`

#### What the issue is

```rust
const DEFAULT_BANK_NAME: &str = "Partner Bank";
...
.bind(DEFAULT_BANK_NAME)  // ← always "Partner Bank"
```

Every recipient saved via `POST /recipients` gets `bank_name = "Partner Bank"` regardless of the `bank_code` provided. The customer's beneficiary list will show "Partner Bank" for every recipient. This is functionally broken — a real payout would fail if the wrong bank is used.

#### How to fix it — step by step

1. **Add `bank_name` to `SaveRecipientBody`** as an optional field. The frontend already knows the bank name from a bank-code lookup table it displays in the UI:
   ```rust
   struct SaveRecipientBody {
       ...
       bank_name: Option<String>,
   }
   ```

2. **Resolve the bank name** from the `bank_code` using a lookup table in code (for the 20-30 major Nigerian and international banks relevant to this corridor), falling back to `"Unknown Bank"` if the code is not recognized.

3. **Add a static `BANK_REGISTRY`** map in `src/domain/recipients.rs`:
   ```rust
   static BANK_REGISTRY: &[(&str, &str)] = &[
       ("044", "Access Bank"),
       ("011", "First Bank of Nigeria"),
       // ... etc
   ];
   ```

4. **Accept the client-provided `bank_name`** if the code is not in the registry. Trim and validate that it is non-empty.

5. **Add tests** asserting that known bank codes resolve to the correct name, and that unknown codes fall back gracefully.

#### Acceptance criteria

- [ ] `POST /recipients` with `bankCode: "044"` saves `bankName: "Access Bank"`.
- [ ] An unknown bank code does not fail — it stores a client-provided name or a fallback.
- [ ] No recipient has `bank_name = "Partner Bank"` unless that is the actual bank name.

---

### ISSUE-30: `validateBankAccount` always returns `"Verified Beneficiary (XXXX)"` — no real name resolution

**Priority:** P2
**Label:** enhancement
**Files:** `src/domain/recipients.rs`

#### What the issue is

```rust
fn resolve_account_name(account_number: &str) -> String {
    format!("Verified Beneficiary ({tail})")
}
```

`POST /recipients/validate` always returns a synthetic "Verified Beneficiary" name derived from the last 4 digits of the account number. It never calls a real name-lookup partner (e.g., NIBSS, Mono, or a payout partner's verification API). This means:
- Customers cannot verify they have the correct beneficiary before saving.
- Fraudulent transfers to wrong accounts cannot be caught at entry.
- The name shown in the transfer confirmation step is fake.

#### How to fix it — step by step

1. **Define a `BankAccountValidator` trait** in `src/domain/recipients.rs`:
   ```rust
   #[async_trait]
   pub trait BankAccountValidator: Send + Sync {
       async fn validate(&self, account_number: &str, bank_code: &str) -> ApiResult<String>;
   }
   ```

2. **Implement `StubValidator`** that returns the current fake name (for dev/tests).

3. **Add a `validator: Arc<dyn BankAccountValidator>` field to `AppState`** (or pass it as a dependency to the domain function).

4. **When a real partner is available**, implement `PartnerValidator` that makes an authenticated HTTP call to the partner's account-name API and returns the real account holder name.

5. **Document in `.env.example`**: which environment variables configure the partner credentials.

#### Acceptance criteria

- [ ] The validator trait is defined and the stub implementation is used in dev/test.
- [ ] `POST /recipients/validate` returns the account holder name from the configured validator.
- [ ] Tests use the stub and are not affected by the trait introduction.
- [ ] The trait can be swapped to a real implementation without changing the route or service code.

---

### ISSUE-31: FX jitter writes to `fx_rates` table on every GET — pollutes data in production

**Priority:** P2
**Label:** bug
**Files:** `src/domain/fx.rs`

#### What the issue is

Every call to `GET /rates/indicative` in production runs:
```rust
let as_of: DateTime<Utc> = sqlx::query_scalar(
    "update fx_rates set rate = $2, as_of = now() where pair = $1 returning as_of",
)
```

This means every indicative rate request mutates the `fx_rates` table. Consequences:
- The `audit_log` trigger (if any audit is added to fx_rates) will generate noise.
- The rate drifts permanently on every read — after 1,000 requests, the rate is far from the seeded value.
- Concurrent requests can write conflicting rates.
- A read-only database replica cannot serve indicative rates.

#### How to fix it — step by step

1. **Separate the "serve a rate" path from the "update the cache" path.** The jitter should be applied in a background task that updates `fx_rates` on a timer (e.g., every 30 seconds in dev), not on every GET.

2. **Update `current_rate`**: remove the `UPDATE` statement. `GET /rates/indicative` becomes a pure read.

3. **Add a background task** in `main.rs` (or a `spawn` in `build_app`) that runs every 30 seconds, applies the jitter, and writes the updated rate back to `fx_rates`. Disable this task in test config (`is_test = true`).

4. **For production**, replace the jitter entirely with a real FX feed pull (e.g., Open Exchange Rates, a payout partner's rate API) in this background task.

#### Acceptance criteria

- [ ] `GET /rates/indicative` does not write to any database table.
- [ ] FX rates are updated by a background task, not by the GET handler.
- [ ] In test config (`is_test = true`), the background task is disabled and rates are static.

---

### ISSUE-32: `change_percent_24h` is a static seed value — never updated after seeding

**Priority:** P2
**Label:** bug
**Files:** `src/seed.rs`, `src/domain/fx.rs`

#### What the issue is

The `fx_rates` table is seeded with static `change_percent_24h` values (e.g., `0.32` for USD/NGN). This value is read and returned on every `GET /rates/indicative` call but is never recalculated. As the `rate` column drifts due to jitter (ISSUE-31), the displayed 24h change becomes increasingly meaningless.

#### How to fix it — step by step

1. **Add a `rate_24h_ago` column** to `fx_rates`:
   ```sql
   alter table fx_rates add column rate_24h_ago numeric(20,8);
   ```

2. **In the background FX update task** (from ISSUE-31), record the current rate as `rate_24h_ago` once per 24-hour cycle and compute `change_percent_24h = (current_rate - rate_24h_ago) / rate_24h_ago * 100`.

3. **For dev/demo**, seed a reasonable `rate_24h_ago` value and let the background task maintain it.

#### Acceptance criteria

- [ ] `changePercent24h` in `GET /rates/indicative` reflects a real calculated percentage change, not a static seed value.
- [ ] The field is `null` or `0` when the rate has no 24-hour history yet.

---

### ISSUE-33: Quote arithmetic uses `f64` in `derive_amounts` — precision loss on large amounts

**Priority:** P2
**Label:** bug
**Files:** `src/domain/quote.rs`

#### What the issue is

```rust
fn derive_amounts(field: QuoteAmountField, amount_minor: i64, rate: f64) -> (i64, i64) {
    match field {
        QuoteAmountField::Send => (amount_minor, (amount_minor as f64 * rate).round() as i64),
        QuoteAmountField::Receive => ((amount_minor as f64 / rate).round() as i64, amount_minor),
    }
}
```

`amount_minor` is cast to `f64` before multiplication. For large `i64` values (e.g., a USD 1,000,000 transfer has `amount_minor = 100_000_000`), `f64` has only 53 bits of mantissa (~15-16 significant decimal digits). At `rate = 1645.2`, the product is `164_520_000_000_000` — 15 digits — at the edge of f64 precision. This is directly related to ISSUE-08 and should be resolved as part of that fix, but is called out separately as the arithmetic function itself needs to change.

#### How to fix it — step by step

1. This issue is resolved as part of ISSUE-08 (migrate to `rust_decimal`). Reference that issue for the full fix.

2. Specifically, `derive_amounts` should become:
   ```rust
   fn derive_amounts(field: QuoteAmountField, amount_minor: i64, rate: Decimal) -> ApiResult<(i64, i64)> {
       let amount = Decimal::from(amount_minor);
       match field {
           QuoteAmountField::Send => {
               let receive = (amount * rate).round().to_i64()
                   .ok_or_else(|| ApiError::validation("Amount overflow."))?;
               Ok((amount_minor, receive))
           }
           QuoteAmountField::Receive => {
               let send = (amount / rate).round().to_i64()
                   .ok_or_else(|| ApiError::validation("Amount overflow."))?;
               Ok((send, amount_minor))
           }
       }
   }
   ```

#### Acceptance criteria

- [ ] `derive_amounts` uses `Decimal` arithmetic (resolved by ISSUE-08).
- [ ] No `f64` cast of `amount_minor` in the quote path.

---

## P2 — Recipients

---

### ISSUE-34: No `GET /ledger/balances` standalone route — dashboard calls it directly

**Priority:** P2
**Label:** enhancement
**Files:** `src/domain/ledger.rs`, `src/lib.rs`

#### What the issue is

`ledger::get_balances` is a domain function called internally by `dashboard::get_overview`. There is no standalone `GET /ledger/balances` HTTP endpoint. The `live-api-client.ts` in `integration/` defines `ledger.getBalances()` as a separate API call, suggesting the frontend expects to be able to fetch account balances independently of the full dashboard overview. Without this route, the frontend cannot refresh balances without re-fetching the entire dashboard payload.

#### How to fix it — step by step

1. **Add a `GET /ledger/balances` route** in `src/domain/ledger.rs` that returns `Vec<AccountBalance>` for the authenticated customer.

2. **Register the route** in `src/lib.rs` by adding a `ledger::routes()` function.

3. **Add an integration test** in `tests/` that calls `GET /ledger/balances` and asserts it returns the seeded opening balances.

#### Acceptance criteria

- [ ] `GET /ledger/balances` returns `200 OK` with a `Vec<AccountBalance>` for the authenticated customer.
- [ ] The response matches the balances returned from `GET /dashboard/overview`.

---

## P3 — Trade Documents (Entire Surface Missing)

---

### ISSUE-35: `GET /transfers/{id}/trade-documents/checklist` not implemented

**Priority:** P3
**Label:** enhancement
**Files:** `src/domain/transfers/` (new file needed)

#### What the issue is

Trade documents (PAAR, Form Q, Bill of Lading, Commercial Invoice, etc.) are a central compliance requirement for Nigerian cross-border exporters. The backend plan (P3) specifies a full trade document checklist endpoint. It does not exist. Without it, the frontend cannot show which documents are required, which have been submitted, and which are pending review. The hardcoded `pending_actions` in the dashboard (ISSUE-25) is a direct symptom of this gap.

#### How to fix it — step by step

1. **Create a `trade_documents` migration**: add a `trade_document_checklists` table and a `trade_documents` table:
   ```sql
   create table trade_document_checklists (
     id           uuid primary key default gen_random_uuid(),
     transfer_id  uuid not null unique references transfers(id) on delete cascade,
     created_at   timestamptz not null default now()
   );

   create table trade_documents (
     id              uuid primary key default gen_random_uuid(),
     checklist_id    uuid not null references trade_document_checklists(id) on delete cascade,
     doc_type        text not null,  -- 'paar', 'form_q', 'bill_of_lading', 'commercial_invoice', etc.
     status          text not null default 'required'
                     check (status in ('required', 'submitted', 'approved', 'replacement_requested')),
     storage_key     text,
     file_name       text,
     submitted_at    timestamptz,
     reviewed_at     timestamptz,
     review_note     text
   );
   ```

2. **Create the checklist when a transfer reaches `AWAITING_FUNDS`** in the engine, seeding the required document types based on the transfer corridor and amount.

3. **Implement `GET /transfers/{id}/trade-documents/checklist`** to return the checklist with item statuses.

4. **Add tests** for checklist creation and retrieval.

#### Acceptance criteria

- [ ] A checklist is created automatically when a transfer reaches `AWAITING_FUNDS`.
- [ ] `GET /transfers/{id}/trade-documents/checklist` returns the required document types and their statuses.
- [ ] A transfer for a new customer with no documents shows all items as `"required"`.

---

### ISSUE-36: `GET /transfers/{id}/trade-documents` not implemented

**Priority:** P3
**Label:** enhancement
**Files:** `src/domain/transfers/` (new file needed)

#### What the issue is

There is no endpoint to list all trade documents that have been uploaded for a specific transfer. The frontend's transfer detail page needs this to show uploaded files, their review status, and any reviewer notes.

#### How to fix it — step by step

1. **Implement `GET /transfers/{id}/trade-documents`** that returns all `trade_documents` rows for the transfer's checklist.

2. **Include file metadata** (name, size, type, status, submitted_at, review_note) but not the file bytes. File download should be a separate signed URL endpoint.

3. **Ownership check**: verify the transfer belongs to the session's customer before returning documents.

4. **Add tests** for the list endpoint.

#### Acceptance criteria

- [ ] `GET /transfers/{id}/trade-documents` returns all documents for that transfer.
- [ ] Documents belonging to another customer's transfer return `404`.

---

### ISSUE-37: `POST /transfers/{id}/trade-documents` not implemented

**Priority:** P3
**Label:** enhancement
**Files:** `src/domain/transfers/` (new file needed)

#### What the issue is

Customers cannot upload trade documents through the API. This endpoint is central to the compliance workflow. Without it, transfers cannot move from `AWAITING_FUNDS` to funded because there is no way to satisfy the document checklist.

#### How to fix it — step by step

1. **Implement `POST /transfers/{id}/trade-documents`** as a multipart form upload, similar to `POST /onboarding/application/documents`.

2. **Accept fields**: `type` (e.g., `"paar"`), `file` (bytes), `applicationId` (optional).

3. **Validate**: allowed MIME types (PDF, JPG, PNG), max 10 MB, transfer must be in a state that accepts document uploads (`AWAITING_FUNDS`, `FUNDED`, `SETTLING`).

4. **Store the file** using `storage::put` with key `trades/{transfer_id}/{doc_type}/{timestamp}-{filename}`.

5. **Transition the checklist item** to `submitted`.

6. **Write an audit log entry.**

7. **Add tests** for successful upload, wrong MIME type, too-large file, and upload after transfer is `COMPLETED` (should be rejected).

#### Acceptance criteria

- [ ] `POST /transfers/{id}/trade-documents` with a valid PDF returns `201 CREATED` with the document record.
- [ ] The checklist item for that `doc_type` transitions to `"submitted"`.
- [ ] Upload on a `COMPLETED` transfer returns `409 CONFLICT`.

---

### ISSUE-38: Trade document status lifecycle not modelled

**Priority:** P3
**Label:** enhancement
**Files:** `src/domain/transfers/` (new file needed)

#### What the issue is

The trade document workflow requires ops reviewers to approve documents or request replacements. The `trade_documents` table structure (from ISSUE-35) has `status` with values `submitted | approved | replacement_requested`, but there are no service functions or routes for ops to change document status. Without this lifecycle, documents are permanently stuck at `submitted` and transfers can never proceed.

#### How to fix it — step by step

1. **Implement `PATCH /ops/transfers/{id}/trade-documents/{doc_id}/review`** (P4 ops surface, see ISSUE-45) that accepts `{ action: "approve" | "request_replacement", note: string }`.

2. **On approve**: set `status = 'approved'`, `reviewed_at = now()`, write audit entry, check if all required documents are now approved and if so emit a notification or trigger an automatic state advance.

3. **On request_replacement**: set `status = 'replacement_requested'`, `review_note = note`, write audit entry, notify customer.

4. **Add tests** for both approval and replacement-request transitions.

#### Acceptance criteria

- [ ] An ops user can approve a submitted document; its status changes to `approved`.
- [ ] An ops user can request a replacement; the customer can re-upload.
- [ ] Approving the last required document triggers the appropriate next action.

---

## P3 — Screening

---

### ISSUE-39: `GET /transfers/{id}/screening` customer-facing endpoint not implemented

**Priority:** P3
**Label:** enhancement
**Files:** `src/domain/transfers/routes.rs`

#### What the issue is

When a transfer is in the `SCREENED` state with `hold: true`, the customer needs to understand why their transfer is held and what the expected resolution timeline is. The plan specifies a `GET /transfers/{id}/screening` endpoint that returns screening status, hold reason, and estimated resolution. This endpoint does not exist. Customers have no way to check their screening status through the API.

#### How to fix it — step by step

1. **Add a `screening_results` table** to store the outcome of the compliance screening:
   ```sql
   create table screening_results (
     id             uuid primary key default gen_random_uuid(),
     transfer_id    uuid not null unique references transfers(id),
     hold           boolean not null default false,
     hold_reason    text,
     resolved_at    timestamptz,
     resolution     text,
     screened_at    timestamptz not null default now()
   );
   ```

2. **Populate this table** in the engine when a transfer transitions to `SCREENED` (currently `payload_for(Screened)` returns `{ "hold": false }` — store this in `screening_results`).

3. **Implement `GET /transfers/{id}/screening`** that returns the screening record for the transfer.

4. **Add tests** for the endpoint.

#### Acceptance criteria

- [ ] `GET /transfers/{id}/screening` returns `{ hold: false, screenedAt: "..." }` for a cleared transfer.
- [ ] A transfer that does not exist returns `404`.
- [ ] A transfer that belongs to another customer returns `404`.

---

### ISSUE-40: Screening result is always `hold: false` — compliance hold path is never exercised

**Priority:** P3
**Label:** bug
**Files:** `src/domain/transfers/engine.rs`

#### What the issue is

In `engine.rs`, `payload_for(Screened)` always returns `json!({ "hold": false })`. There is no code path that produces `hold: true`. This means:
- The `ComplianceHold` error code is defined in `error.rs` but never used.
- A high-risk transfer (e.g., to a sanctioned beneficiary) would pass screening silently.
- The `Screened.hold` field is effectively dead code.

#### How to fix it — step by step

1. **Add a stub screening function** in a new `src/domain/screening.rs` module (similar to `kyb.rs`), with documented trigger conditions for testing:
   - Transfer to a recipient whose `country` is in a watchlist (e.g., `"IR"`, `"KP"`) → `hold: true`.
   - Transfer above a threshold amount (e.g., USD 500,000) → `hold: true` for manual review.

2. **Call the screening function** from `engine::advance_once` when transitioning to `Screened`.

3. **If `hold: true`**: set the payload with `expectedResolutionBy` (48 hours from now), do NOT auto-advance to `AWAITING_FUNDS`.

4. **Add an ops route** `POST /ops/transfers/{id}/screening/decision` (see ISSUE-46) to clear or reject the hold.

5. **Add tests**: a transfer to a watchlisted country enters `hold: true`; an ops decision clears it.

#### Acceptance criteria

- [ ] A transfer to a watchlisted country results in `SCREENED` with `hold: true`.
- [ ] The transfer does not auto-advance to `AWAITING_FUNDS` while `hold: true`.
- [ ] An ops decision clears the hold and advances the transfer.
- [ ] All existing tests for non-held transfers still pass.

---

## P4 — Ops Back-Office Surface

---

### ISSUE-41: `GET/POST /ops/transactions` ops search and action endpoints not implemented

**Priority:** P4
**Label:** enhancement
**Files:** `src/domain/` (new ops module needed)

#### What the issue is

The entire ops back-office API surface is missing. Operators have no way to search transfers, view transaction details, or take actions (initiate/approve/reject) through the API. This is a blocking gap for any real operational use of the platform.

#### How to fix it — step by step

1. **Create `src/domain/ops/mod.rs`** with sub-modules: `transactions.rs`, `screening.rs`, `reconciliation.rs`, `audit.rs`, `partners.rs`.

2. **Add a role guard middleware** for all `/ops/*` routes: reject any `Session` where `session.role != "operator"` with `403 FORBIDDEN`.

3. **Implement `GET /ops/transactions`** with filters: `status`, `customer_id`, `reference`, `date range`, `currency`, `amount_min`, `amount_max`. Return paginated results.

4. **Implement `GET /ops/transactions/{id}`** for full transfer detail including the internal `note` from `transfer_state_history`.

5. **Implement `POST /ops/transactions/{id}/action`** with body `{ action: "approve" | "reject", note: string }` to manually advance or reject a transfer.

6. **Add integration tests** with a test session that has `role = 'operator'`.

#### Acceptance criteria

- [ ] Calling any `/ops/*` route with `role = 'customer'` returns `403 FORBIDDEN`.
- [ ] `GET /ops/transactions` returns a filtered, paginated list of transfers.
- [ ] An operator can reject a transfer via `POST /ops/transactions/{id}/action`.
- [ ] The rejection writes an ops-only note to `transfer_state_history`.

---

### ISSUE-42: `GET/POST /ops/screening/queue` and `/decision` not implemented

**Priority:** P4
**Label:** enhancement
**Files:** `src/domain/ops/screening.rs` (new file needed)

#### What the issue is

Transfers that enter `SCREENED` with `hold: true` (ISSUE-40) need to be reviewed by an ops compliance analyst. Without a screening queue endpoint, there is no way for an operator to discover which transfers are on hold, review the reason, and make a compliance decision. Held transfers will sit indefinitely.

#### How to fix it — step by step

1. **Implement `GET /ops/screening/queue`**: return all transfers with `SCREENED` status where `payload->>'hold' = 'true'`, paginated, with `entered_at` (how long they have been held).

2. **Implement `POST /ops/screening/{transfer_id}/decision`** with body `{ decision: "clear" | "reject", reason: string }`:
   - `"clear"`: update `screening_results.hold = false`, advance the transfer to `AWAITING_FUNDS`.
   - `"reject"`: transition the transfer to `REJECTED` with `failureCategory: "complianceHold"`.

3. **Write an audit log entry** for every decision.

4. **Add tests** for both clear and reject decisions.

#### Acceptance criteria

- [ ] `GET /ops/screening/queue` returns all held transfers.
- [ ] A `clear` decision advances the transfer to `AWAITING_FUNDS`.
- [ ] A `reject` decision moves the transfer to `REJECTED`.
- [ ] All decisions are recorded in `audit_log`.

---

### ISSUE-43: `GET/PATCH /ops/reconciliation` endpoints not implemented

**Priority:** P4
**Label:** enhancement
**Files:** `src/domain/ops/reconciliation.rs` (new file needed)

#### What the issue is

Reconciliation is the process of matching the platform's internal ledger against the payout partner's transaction records. Without reconciliation endpoints, finance cannot identify discrepancies (e.g., a transfer marked `COMPLETED` internally but not settled by the partner, or a partner payout with no matching internal transfer). This is a critical operational gap for any real financial platform.

#### How to fix it — step by step

1. **Create a `reconciliation_breaks` table**:
   ```sql
   create table reconciliation_breaks (
     id            uuid primary key default gen_random_uuid(),
     transfer_id   uuid references transfers(id),
     break_type    text not null,   -- 'missing_in_partner', 'missing_internally', 'amount_mismatch'
     expected      jsonb,
     actual        jsonb,
     status        text not null default 'open' check (status in ('open', 'resolved', 'disputed')),
     resolved_at   timestamptz,
     resolution    text,
     created_at    timestamptz not null default now()
   );
   ```

2. **Implement `GET /ops/reconciliation`**: return open reconciliation breaks, paginated.

3. **Implement `PATCH /ops/reconciliation/{id}`**: allow an operator to mark a break as `resolved` or `disputed` with a `resolution` note.

4. **Stub a reconciliation import**: `POST /ops/reconciliation/import` that accepts a CSV of partner transactions and compares against internal `transfers`, creating break records for discrepancies.

#### Acceptance criteria

- [ ] `GET /ops/reconciliation` returns all open reconciliation breaks.
- [ ] An operator can resolve or dispute a break via `PATCH`.
- [ ] All state changes are audit-logged.

---

### ISSUE-44: `GET /ops/audit` paginated audit log read not implemented

**Priority:** P4
**Label:** enhancement
**Files:** `src/domain/ops/audit.rs` (new file needed)

#### What the issue is

The `audit_log` table is append-only and grows with every operation. There is no API endpoint to read it. Operators have no way to review the audit trail for a specific transfer or customer without direct database access. This is a compliance requirement for financial services.

#### How to fix it — step by step

1. **Implement `GET /ops/audit`** with query params: `entity_type`, `entity_id`, `actor_id`, `from`, `to`, `limit`, `before` (cursor).

2. **Return paginated `audit_log` rows** in reverse chronological order.

3. **Apply role guard**: ops only.

4. **Add a test** that generates several audit events and retrieves them via the endpoint.

#### Acceptance criteria

- [ ] `GET /ops/audit?entity_type=transfer&entity_id=<id>` returns all audit events for that transfer.
- [ ] Results are paginated and in reverse chronological order.
- [ ] Non-operators receive `403`.

---

### ISSUE-45: `GET /ops/partners` partner health status endpoint not implemented

**Priority:** P4
**Label:** enhancement
**Files:** `src/domain/ops/partners.rs` (new file needed)

#### What the issue is

The platform depends on external partners (KYB provider, FX feed, payout partner, bank name resolver). When a partner is degraded or down, operators need to know immediately. There is no partner health status endpoint, so operators must check partner dashboards manually.

#### How to fix it — step by step

1. **Define a `PartnerStatus` struct**:
   ```rust
   pub struct PartnerStatus {
       pub name: String,
       pub status: &'static str,   // "operational" | "degraded" | "down"
       pub last_checked: String,
       pub latency_ms: Option<u64>,
   }
   ```

2. **Implement `GET /ops/partners`** that returns a list of `PartnerStatus` for each configured integration. For now, return stub statuses (all `"operational"`). When real integrations land, perform a lightweight health check (e.g., a ping or a minimal API call) and cache the result for 60 seconds.

3. **Add a test** that calls the endpoint and asserts the response shape.

#### Acceptance criteria

- [ ] `GET /ops/partners` returns a list of partners with their status, last-checked time, and latency.
- [ ] Non-operators receive `403`.
- [ ] The endpoint responds within 500 ms even when all partner checks are performed.

---

## P4 — Infrastructure and Cross-Cutting

---

### ISSUE-46: File storage uses local filesystem only — no S3/MinIO integration

**Priority:** P4
**Label:** enhancement
**Files:** `src/storage.rs`

#### What the issue is

`src/storage.rs` implements `put` and `delete` using `tokio::fs` — files are stored on the local disk under the `STORAGE_DIR` path. This works for a single-server development setup but fails in any distributed or containerized production environment where:
- Multiple server instances cannot share a local filesystem.
- Container restarts delete the storage directory.
- Files cannot be served to clients via signed URLs.
- Disaster recovery requires object storage with versioning and replication.

The comment in `storage.rs` notes that `"A MinIO/S3 impl slots in behind the same three functions"` — the abstraction exists, but no S3 implementation has been written.

#### How to fix it — step by step

1. **Define a `Storage` trait** in `src/storage.rs`:
   ```rust
   #[async_trait]
   pub trait Storage: Send + Sync {
       async fn put(&self, key: &str, bytes: &[u8]) -> ApiResult<()>;
       async fn delete(&self, key: &str) -> ApiResult<()>;
       async fn presigned_get_url(&self, key: &str, expires_in_secs: u64) -> ApiResult<String>;
   }
   ```

2. **Implement `LocalStorage`** wrapping the current `tokio::fs` logic (for dev).

3. **Implement `S3Storage`** using the `aws-sdk-s3` crate:
   - Configure via `AWS_REGION`, `S3_BUCKET`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` env vars.
   - `put`: `PutObject`.
   - `delete`: `DeleteObject`.
   - `presigned_get_url`: generate a presigned `GetObject` URL.

4. **Add `storage: Arc<dyn Storage>` to `AppState`** and inject based on `STORAGE_BACKEND=local|s3` env var.

5. **Update all callers** (`src/domain/onboarding/service.rs` and future trade document service) to use `state.storage` instead of calling `storage::put/delete` directly.

6. **Add document download endpoints** for onboarding docs and trade docs that generate presigned URLs.

7. **Update `.env.example`** with `STORAGE_BACKEND`, `S3_BUCKET`, and AWS credential documentation.

#### Acceptance criteria

- [ ] `STORAGE_BACKEND=local` uses the filesystem (current behavior, default for dev).
- [ ] `STORAGE_BACKEND=s3` stores and retrieves files from S3/MinIO.
- [ ] `GET /onboarding/application/documents/{id}/download` returns a presigned URL valid for 15 minutes.
- [ ] Switching backends requires only an env var change, not a code change.

---

### ISSUE-47: No `POST /auth/register` or multi-tenant signup flow

**Priority:** P4
**Label:** enhancement
**Files:** `src/domain/` (new auth module), `src/http/auth.rs`

#### What the issue is

Currently there is only one customer in the system (the seeded demo). There is no way for a new business to sign up without running `cargo run --bin seed`, which would truncate all existing data. The platform cannot onboard real customers until a registration flow exists. This is blocked by ISSUE-01 (real login), but the registration flow itself is also absent.

#### How to fix it — step by step

1. **Implement `POST /auth/register`** that accepts `{ email, password, legalName }` (no session required):
   - Validate email format and uniqueness.
   - Hash the password with bcrypt.
   - Create a `users` row, a `customers` row, and an `onboarding_applications` row in one transaction.
   - Return a session token (same as login response).

2. **Create a `POST /auth/verify-email`** stub that accepts a token from an email verification link and marks the user as verified.

3. **Gate certain API operations** on email verification (e.g., submitting an onboarding application).

4. **Add tests**: register a new user, verify they can log in, verify their onboarding application is created in `draft` state.

#### Acceptance criteria

- [ ] `POST /auth/register` creates a user, customer, and draft onboarding application in one transaction.
- [ ] Duplicate email returns `409 CONFLICT`.
- [ ] The newly registered user can log in with `POST /auth/login`.
- [ ] The demo seed (`cargo run --bin seed`) continues to work for development.

---

### ISSUE-48: No database connection pool health check or graceful reconnect

**Priority:** P4
**Label:** enhancement
**Files:** `src/db.rs`, `src/main.rs`

#### What the issue is

`src/db.rs` creates a `PgPool` and runs migrations, but there is no health check endpoint that verifies the database connection is alive, no pool monitoring, and no graceful reconnect logic. If the Postgres server is temporarily unavailable (e.g., during a failover or maintenance window), all requests fail with generic `SERVER_ERROR` responses. Operators have no way to distinguish a database outage from an application bug via the `/health` endpoint, which currently always returns `{ "ok": true }` regardless of DB state.

#### How to fix it — step by step

1. **Update `GET /health`** to include a database liveness check:
   ```rust
   async fn health(State(state): State<AppState>) -> impl IntoResponse {
       let db_ok = sqlx::query("select 1").execute(&state.pool).await.is_ok();
       let status = if db_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
       (status, Json(json!({ "ok": db_ok, "db": db_ok })))
   }
   ```

2. **Add pool configuration** in `src/db.rs`:
   - `max_connections`: configurable via `DATABASE_MAX_CONNECTIONS` env var (default 10).
   - `acquire_timeout`: 5 seconds (return `SERVER_ERROR` rather than hanging).
   - `idle_timeout`: 10 minutes.
   - `max_lifetime`: 30 minutes.

3. **Add a `GET /health/ready`** endpoint (readiness probe for Kubernetes) that returns `503` until the pool is connected and migrations have run.

4. **Add a `GET /health/live`** endpoint (liveness probe) that returns `200` as long as the process is running.

#### Acceptance criteria

- [ ] `GET /health` returns `503` when the database is unreachable.
- [ ] `GET /health/ready` returns `503` before the pool is ready and `200` after.
- [ ] Database connection pool size is configurable via environment variable.

---

### ISSUE-49: `live-api-client.ts` in `integration/` has not been applied to the frontend

**Priority:** P2
**Label:** tech-debt
**Files:** `Kimana_backend/integration/live-api-client.ts`, `Kimana_frontend/src/api/index.js`

#### What the issue is

`Kimana_backend/integration/live-api-client.ts` is a complete drop-in replacement for the frontend's mock API client. The `integration/README.md` says it should be copied into the frontend and wired in. However, `Kimana_frontend/src/api/index.js` still exports the mock client:

```js
// src/api/index.js
import { mockApiClient } from './mock'
export const api = mockApiClient
```

None of the real backend endpoints are called by the frontend. Every dashboard, onboarding, transfer, and FX operation still runs against the in-memory mock. The 49 passing backend integration tests are invisible to the frontend's actual user flow.

#### How to fix it — step by step

1. **Copy `live-api-client.ts`** into `Kimana_frontend/src/api/live.ts`.

2. **Update `src/api/index.js`** to select the client based on an environment variable:
   ```js
   const useLive = process.env.NEXT_PUBLIC_API_MODE === 'live'
   export const api = useLive ? liveApiClient : mockApiClient
   ```

3. **Set `NEXT_PUBLIC_API_URL=http://localhost:4000`** and `NEXT_PUBLIC_API_MODE=live` in `.env.local`.

4. **Fix any type mismatches** between what `live-api-client.ts` expects and what the backend returns (date formats, field names, etc.).

5. **Test the full flow** with the backend running: login → dashboard → create transfer → check status.

6. **Update `NewTransferModal`** in the frontend to call the real API instead of the local simulation.

#### Acceptance criteria

- [ ] With `NEXT_PUBLIC_API_MODE=live`, all frontend operations call the real backend.
- [ ] Dashboard, FX rates, onboarding, recipients, quotes, and transfers all work end-to-end with the backend running.
- [ ] The mock client remains available for `NEXT_PUBLIC_API_MODE=mock` (default, for offline dev).

---

### ISSUE-50: No rate limiting or request throttling on any endpoint

**Priority:** P4
**Label:** enhancement
**Files:** `src/lib.rs`

#### What the issue is

There is no rate limiting on any endpoint. An attacker or misbehaving client can:
- Hammer `POST /auth/login` with credential-stuffing attacks.
- Flood `POST /quotes` to exhaust the database with quote rows.
- Send thousands of `POST /transfers` requests to generate reference collisions (see ISSUE-07).
- Cause the KYB `submit` endpoint to spawn thousands of background tasks.

Without rate limiting, a single misbehaving client can degrade the service for all other customers.

#### How to fix it — step by step

1. **Add `tower_governor`** to `Cargo.toml`:
   ```toml
   tower_governor = "0.4"
   ```

2. **Apply a global rate limit layer** in `build_app` in `src/lib.rs`:
   ```rust
   use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
   let governor_conf = GovernorConfigBuilder::default()
       .per_second(100)          // global: 100 requests/second per IP
       .burst_size(200)
       .finish().unwrap();
   ```

3. **Apply a tighter limit to sensitive endpoints** (`/auth/login`, `/onboarding/application/submit`, `/quotes`) using per-route governor layers: 5 requests per second per IP.

4. **Return `429 Too Many Requests`** with a `Retry-After` header when the limit is exceeded.

5. **Disable rate limiting in test config** (`is_test = true`) to avoid slowing down tests.

6. **Document limits** in `README.md`.

#### Acceptance criteria

- [ ] `POST /auth/login` returns `429` after 5 rapid requests from the same IP.
- [ ] `GET /health` and `GET /rates/indicative` are not blocked by the tight rate limit.
- [ ] Integration tests are not affected (rate limiting disabled in test config).
- [ ] `429` responses include a `Retry-After` header indicating when the client may retry.

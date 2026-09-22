# Kimana Backend — Security Remediation Issues

This document outlines the actionable security issues and engineering fixes identified during the Static Application Security Testing (SAST) and architecture audit of `Kimana_backend`. Each issue includes exact file locations, technical root causes, step-by-step resolution guides, code patches, and testable acceptance criteria.

---

## Issue Summary

| Issue ID | Severity | Target Module | OWASP Category | Description |
| :--- | :---: | :--- | :--- | :--- |
| **`ISSUE-BE-01`** | **CRITICAL** | `src/http/auth.rs:35` | OWASP A01 / A07 | Session extractor always resolves to hardcoded `DEMO_USER_ID`. |
| **`ISSUE-BE-02`** | **HIGH** | `src/domain/quote.rs:93` | OWASP A04 (Design) | Floating-point `f64::round()` causes on-chain settlement reverts (`ReceiveAmountMismatch`). |
| **`ISSUE-BE-03`** | **MEDIUM** | `Cargo.lock` | OWASP A06 (Supply Chain) | Known vulnerability in `rustls v0.23.43` (`RUSTSEC-2026-0285`). |
| **`ISSUE-BE-04`** | **MEDIUM** | `src/domain/transfers` | OWASP A08 (Integrity) | Single quote can be consumed by multiple transfers concurrently. |
| **`ISSUE-BE-05`** | **MEDIUM** | `src/lib.rs:55` | OWASP A04 (DoS) | Global 12 MB body limit without request timeouts or rate-limiting layers. |
| **`ISSUE-BE-06`** | **LOW** | `src/domain/*/repo.rs` | OWASP A03 (Injection) | Dynamic string formatting (`format!`) used in SQL queries. |
| **`ISSUE-BE-07`** | **MEDIUM** | `src/error.rs`, `src/lib.rs` | CWE-209 (Observability) | Error responses carried no request correlation id for support/log cross-referencing. |

---

### ISSUE-BE-01: Implement Production Authentication & Token Validation

**Status:** ✅ Resolved in `d672d55` ("feat: add real user registration, login, and DB-backed sessions"). `Session::from_request_parts` now reads the `kimana_session` cookie, hashes it, and requires a live (non-expired, non-revoked) row in `sessions`; there is no unauthenticated fallback. See `src/http/auth.rs`.

**Priority:** Critical (P0)  
**Labels:** `security`, `auth`, `access-control`  
**Files:** `src/http/auth.rs`, `src/routes.rs`, `migrations/008_sessions.sql`  

#### 1. What the Problem Is
`Session::from_request_parts` in `src/http/auth.rs` unconditionally loads the row matching `DEMO_USER_ID`:
```rust
let row: Option<SessionRow> = sqlx::query_as(
    "select u.id as user_id, u.role, u.display_name,
            u.operator_permissions, c.id as customer_id
       from users u
       join customers c on c.primary_user_id = u.id
      where u.id = $1",
)
.bind(DEMO_USER_ID)
.fetch_optional(&state.pool)
.await?;
```
Any incoming request without cookies, tokens, or credentials inherits full administrative and financial authority over the demo customer's balance, recipients, and transfers.

#### 2. Step-by-Step Resolution Guide
1. **Add Migration for Sessions (`migrations/008_sessions.sql`):**
   ```sql
   create table sessions (
     id          uuid primary key default gen_random_uuid(),
     user_id     uuid not null references users(id) on delete cascade,
     token_hash  text not null unique,
     created_at  timestamptz not null default now(),
     expires_at  timestamptz not null,
     revoked_at  timestamptz
   );
   create index sessions_token_hash_idx on sessions(token_hash);
   ```
2. **Implement Token Hashing & Validation in `src/http/auth.rs`:**
   Read the `Authorization: Bearer <token>` or `Cookie: session=<token>` header. Compute SHA-256 of the token, and look up the active session:
   ```rust
   let token_hash = sha256_hex(raw_token);
   let row: Option<SessionRow> = sqlx::query_as(
       "select u.id as user_id, u.role, u.display_name,
               u.operator_permissions, c.id as customer_id
          from sessions s
          join users u on u.id = s.user_id
          join customers c on c.primary_user_id = u.id
         where s.token_hash = $1
           and s.expires_at > now()
           and s.revoked_at is null",
   )
   .bind(&token_hash)
   .fetch_optional(&state.pool)
   .await?;
   ```
3. **Return 401 Unauthorized When Missing or Invalid:**
   If `row` is `None`, return `Err(ApiError::unauthorized("Valid authentication credentials required."))`.
4. **Seed / Test Fallback:**
   In development or automated test mode (`state.config.is_test`), permit an explicit `X-Demo-Session: true` header or seed a session token, but never allow silent unauthenticated fallback.

#### 3. Acceptance Criteria
- [x] Requests without valid bearer tokens return `401 Unauthorized`.
- [x] Requests with expired or revoked session tokens return `401 Unauthorized`.
- [x] Valid tokens extract the authentic user's `Session` and `customer_id`.

---

### ISSUE-BE-02: Port Integer Floor FX Math to Synchronize with Smart Contracts

**Status:** ✅ `derive_amounts` (`src/domain/quote.rs`) no longer uses `f64::round()`. It now goes through `apply_rate`/`invert_rate` (`src/util.rs`), which scale the rate to a fixed-point `i128` (`RATE_DECIMALS = 8`) once and then do plain integer floor division for the actual money math — the same `.5`-rounds-up-vs-floors discrepancy this issue describes is covered by `util::rate_math_tests::floors_instead_of_rounding_half_up` and `domain::quote::tests::no_float_round_in_money_math`. **Not independently verified:** there is no `FxMath.sol` / `SettlementVault` in this repository, so "matches `FxMath.sol` across 1,000 test vectors" couldn't be checked here — if that contract lives in another repo, someone with access to it should confirm the scaling (`RATE_DECIMALS`/`USDC_DECIMALS` conventions) actually lines up before relying on this for on-chain reconciliation.

**Priority:** High (P1)  
**Labels:** `security`, `math`, `settlement`  
**Files:** `src/domain/quote.rs`, `src/contract/quote.rs`  

#### 1. What the Problem Is
In `src/domain/quote.rs`, receive amounts are derived using IEEE 754 floating point arithmetic:
```rust
fn derive_amounts(field: QuoteAmountField, amount_minor: i64, rate: f64) -> (i64, i64) {
    match field {
        QuoteAmountField::Send => (amount_minor, (amount_minor as f64 * rate).round() as i64),
        QuoteAmountField::Receive => ((amount_minor as f64 / rate).round() as i64, amount_minor),
    }
}
```
In contrast, `FxMath.sol` on Ethereum/EVM enforces integer floor division:
```solidity
return (usdcAmount * rate * (10 ** receiveDecimals)) / (10 ** (USDC_DECIMALS + RATE_DECIMALS));
```
Whenever the fractional product has a fractional part $\ge 0.5$, Rust's `.round()` rounds up. The smart contract's `SettlementVault.lockQuote()` compares the quote against `FxMath.receiveAmount` and reverts with `ReceiveAmountMismatch`. Half of customer quotes will fail on-chain settlement!

#### 2. Step-by-Step Resolution Guide
1. **Define Scaled Integer Math in `src/util.rs` or `src/domain/fx.rs`:**
   ```rust
   pub const RATE_DECIMALS: u32 = 8;
   pub const USDC_DECIMALS: u32 = 6;

   pub fn calculate_receive_amount(
       usdc_minor: i64,
       rate_scaled8: u128,
       receive_decimals: u8,
   ) -> i64 {
       // Convert backend minor (cents, 2 dec) to USDC base (6 dec): * 10,000
       let usdc_base = (usdc_minor as u128) * 10_000;
       let multiplier = 10u128.pow(receive_decimals as u32);
       let divisor = 10u128.pow(USDC_DECIMALS + RATE_DECIMALS);
       let receive_minor = (usdc_base * rate_scaled8 * multiplier) / divisor;
       receive_minor as i64
   }
   ```
2. **Update `derive_amounts`:**
   Remove floating-point multiplication and use `calculate_receive_amount` matching the target currency's exponent (e.g. NGN = 2, XOF = 0).
3. **Add Property Tests:**
   Add unit tests verifying that Rust calculations match `FxMath.sol` test vectors exactly across 1,000 test cases.

#### 3. Acceptance Criteria
- [x] No floating point `f64::round()` used in money calculation.
- [ ] Receive amount calculation matches `FxMath.sol` across all test vectors with zero difference. *(Unverifiable here — no `FxMath.sol` in this repo; see Status above.)*

---

### ISSUE-BE-03: Update Vulnerable Dependency `rustls`

**Priority:** Medium (P2)  
**Labels:** `dependencies`, `security`  
**Files:** `Cargo.lock`  

#### 1. What the Problem Is
`Cargo.lock` contains `rustls v0.23.43`, which is vulnerable to `RUSTSEC-2026-0285` (GHSA-2mjx-qc3c-rqvc). Rustls accepted TLS 1.3 handshake messages sent at the wrong encryption level across record boundaries.

#### 2. Step-by-Step Resolution Guide
1. Update `rustls` in `Cargo.lock` to the patched release:
   ```bash
   cargo update -p rustls --precise 0.23.45
   ```
2. Run `cargo check` and `cargo test` to verify build integrity.

#### 3. Acceptance Criteria
- [ ] `rustls` version in `Cargo.lock` is `>= 0.23.45`.
- [ ] `cargo audit` reports zero advisories for `rustls`.

---

### ISSUE-BE-04: Atomic Quote Consumption & Single-Use Lock

**Priority:** Medium (P2)  
**Labels:** `bug`, `concurrency`, `transfers`  
**Files:** `src/domain/transfers/service.rs`, `migrations/009_quote_consumption.sql`  

#### 1. What the Problem Is
In `create_transfer()`, quotes are read from the database, but are not atomically marked as consumed. A user can trigger multiple parallel transfer creation requests using the same `quote_id`. While the backend database creates multiple transfer rows, the smart contract strictly reverts on duplicate quote IDs (`QuoteAlreadyUsed`). Only the first transfer settles; all others fail.

#### 2. Step-by-Step Resolution Guide
1. **Add Migration:**
   ```sql
   alter table quotes add column consumed_by_transfer_id uuid references transfers(id);
   ```
2. **Atomic Update in `create_transfer` Transaction:**
   ```rust
   let rows_affected = sqlx::query(
       "update quotes
           set consumed_by_transfer_id = $2
         where id = $1 and consumed_by_transfer_id is null",
   )
   .bind(quote.id)
   .bind(transfer_id)
   .execute(&mut *tx)
   .await?
   .rows_affected();

   if rows_affected == 0 {
       return Err(ApiError::conflict("This quote has already been accepted by another transfer."));
   }
   ```

#### 3. Acceptance Criteria
- [ ] Concurrent transfer attempts using the same `quote_id` reject duplicates with `409 Conflict`.
- [ ] Each firm quote is bound to exactly one transfer.

---

### ISSUE-BE-05: Denial of Service Hardening: Rate Limiting & Timeout Layers

**Status:** ✅ Timeout and route-specific body limits landed (`src/lib.rs`, `src/domain/onboarding/routes.rs`). The default `DefaultBodyLimit` dropped from 12 MB to 128 KB; the document upload route (`POST /onboarding/application/documents` — the issue said `/onboarding/documents`, which doesn't exist) gets its own 12 MB override via `.layer(DefaultBodyLimit::max(...))` on that one route, per axum's documented per-route pattern. A `tower_http::timeout::TimeoutLayer` aborts any request running past 15s with `408`. Verified live: an 8 KB body dripped at 500 B/s returned `408` at 15.0s; a 200 KB JSON body was rejected (`400`, via the existing `Body` extractor's rejection handling — not literally `413`, but rejected as the criterion asks); a 500 KB multipart upload to the document route sailed past the body-limit layer untouched. **Not in scope / still open:** no rate limiting (the issue's title mentions it, but the resolution steps and acceptance criteria only cover timeouts and body limits — flagging in case per-IP/per-session rate limiting was actually wanted too).

**Priority:** Medium (P2)  
**Labels:** `security`, `dos`, `middleware`  
**Files:** `src/lib.rs`, `Cargo.toml`  

#### 1. What the Problem Is
In `src/lib.rs`, the router applies `DefaultBodyLimit::max(12 * 1024 * 1024)` globally across all routes, but has no request timeout middleware and no rate limiting. Slow client requests or high-frequency automated floods can exhaust Tokio worker threads and database connections.

#### 2. Step-by-Step Resolution Guide
1. **Add Middleware Dependencies in `Cargo.toml`:**
   ```toml
   tower-http = { version = "0.6", features = ["cors", "timeout", "limit"] }
   ```
2. **Add Request Timeout and Route-Specific Limits:**
   In `src/lib.rs`:
   ```rust
   use tower_http::timeout::TimeoutLayer;
   use std::time::Duration;

   Router::new()
       // ... routes ...
       .layer(TimeoutLayer::new(Duration::from_secs(15)))
   ```
3. Restrict large 12 MB body limits only to the onboarding document upload route (`/onboarding/documents`), and set a default 128 KB body limit for standard JSON endpoints.

#### 3. Acceptance Criteria
- [x] Requests hanging beyond 15 seconds terminate with `408 Request Timeout`.
- [x] Standard JSON endpoints reject payloads larger than 128 KB.

---

### ISSUE-BE-06: Remediate Dynamic SQL String Formatting in Repository Queries

**Status:** ✅ Every `format!`-built SQL string in `src/domain/transfers/repo.rs` and `src/domain/quote.rs` is now a `const &str` built at compile time via `concat!` over a `macro_rules!` fragment (`transfer_select!`/`quote_cols!`), instead of a runtime `format!` call. No SQL text is assembled at runtime anymore; every user-supplied value still only ever reaches the query through `.bind(...)`, exactly as before — this was never a real injection path (the interpolated pieces were always internal `const` literals), so the fix is about removing the anti-pattern the issue calls out, not about closing an actual injection. Full test suite (79 tests) still passes.

**Priority:** Low (P3)  
**Labels:** `security`, `database`, `sqlx`  
**Files:** `src/domain/transfers/repo.rs`, `src/domain/quote.rs`  

#### 1. What the Problem Is
SQL queries in repository modules use runtime string interpolation:
```rust
let sql = format!("{SELECT_TRANSFERS} where customer_id = $1 order by created_at desc");
```
While the interpolated constants are currently internal strings, dynamic query generation bypasses SQLx compile-time query verification (`sqlx::query!`) and establishes a hazardous anti-pattern that future changes might accidentally extend to user inputs.

#### 2. Step-by-Step Resolution Guide
1. Replace runtime `format!` calls with static string constants or `sqlx::query!` macros.
2. For dynamic filtering, construct query builders or use structured parameter binding.

#### 3. Acceptance Criteria
- [x] No `format!` string construction used for SQL query generation in repository layers.
- [x] All database queries utilize static query strings with parameterized bind arguments.

---

### ISSUE-BE-07: Attach Opaque `request_id` to All Error Responses and Tracing Spans

**Status:** ✅ `SetRequestIdLayer`/`PropagateRequestIdLayer` (`tower_http::request_id`, `MakeRequestUuid`) assign a UUID per request and put it on the `x-request-id` response header for *every* response (success, business error, even the empty-body `408` from `TimeoutLayer` — verified live). A new `attach_request_id` middleware (`src/lib.rs`) additionally (a) runs the request inside a `tracing::info_span!("request", request_id = ...)`, so any `tracing::error!` anywhere in the call chain — e.g. the `sqlx::Error` log in `error.rs`'s `From` impl — is correlated automatically by the tracing span context, with no change needed to that log call itself, and (b) merges a `requestId` field into JSON error bodies, since `ApiError::into_response` has no access to the request to do that itself. Verified live: a `401` body and its `x-request-id` header carry the identical UUID. **One deliberate deviation from the issue's example:** the JSON field is `requestId` (camelCase), not `request_id` — every other multi-word field in this API's JSON contracts (`contract/*.rs`) is `#[serde(rename_all = "camelCase")]`, so `request_id` would have been the one inconsistent field.

**Priority:** Medium (P2)  
**Labels:** `backend`, `security`, `cwe-209`, `observability`, `p2`  
**Files:** `src/error.rs`, `src/lib.rs`  

#### 1. Problem Description
`ApiError::into_response` rendered `{ "code": ..., "message": ..., "retryable": ... }` with no request correlation id, so support couldn't match a customer-reported error to server-side logs without asking for sensitive operational detail.

#### 2. Step-by-Step Implementation Guide
1. Use `tower_http::request_id::MakeRequestUuid` to assign a UUID to every incoming request.
2. Update `ApiError::into_response` to include `request_id`.
3. Record the same id in `tracing::error!(request_id = %req_id, ...)` so server logs can be cross-referenced.

#### 3. Acceptance Criteria
- [x] Every 4xx and 5xx API response includes a request id field (`requestId` in the JSON body where one exists; `x-request-id` header on every response, including the handful of error responses with no JSON body).
- [x] Database error details are logged strictly on the server alongside the matching request id (via the tracing span, not a literal `tracing::error!(request_id = ...)` call — same outcome, no per-call-site plumbing).


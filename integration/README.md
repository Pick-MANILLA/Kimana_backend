# Wiring the frontend to this backend

Step 6 of the P1 slice (`docs/backend-plan.md` §07). These changes live in
**`Kimana_frontend`**, not this repo — they're kept here so the slice is
self-documenting.

## 1. Add the live client

Copy [`live-api-client.ts`](./live-api-client.ts) to:

```
Kimana_frontend/src/api/live/client.ts
```

It implements the slice endpoints (auth, onboarding, dashboard) over HTTP and
**delegates every other method to the existing mock** via `...mockApiClient`.
So the transfer table, recipients, and FX panel keep working against seeded
mock data until P2 replaces them.

## 2. Flip the switch

In `Kimana_frontend/src/api/index.ts`:

```diff
-import { mockApiClient } from './mock';
+import { mockApiClient } from './mock';
+import { createLiveApiClient } from './live/client';

-export const api: ApiClient = mockApiClient;
+export const api: ApiClient =
+  import.meta.env.VITE_API_URL || import.meta.env.PROD
+    ? createLiveApiClient()
+    : mockApiClient;
```

(Keep `export { DEMO_CUSTOMER_ID } from './mock/seed';` — the live client
ignores the `customerId` argument, but the constant is still imported by
screens.)

## 3. Point it at the backend

`Kimana_frontend/.env.local`:

```
VITE_API_URL=http://localhost:4000
```

## 4. Run both

```
# terminal 1 — this repo
docker compose up -d && npm run seed && npm run dev      # :4000

# terminal 2 — Kimana_frontend
npm run dev                                              # :5173
```

Then walk `/onboarding/business-details` → `/onboarding/approved` and land on
`/dashboard`. The onboarding wizard and the dashboard header, balances, stats,
and pending actions are now served by `Kimana_backend`; every onboarding
mutation writes an `audit_log` row.

## Notes

- `CORS_ORIGIN` in the backend `.env` must match the Vite origin
  (`http://localhost:5173` by default).
- `uploadDocument`'s `onProgress` jumps straight to 100 — `fetch` can't stream
  upload progress. Swap to `XMLHttpRequest` there if the progress bar matters.
- The backend uses a single seeded demo session, so there's no login step yet.

# `src/contract` — vendored API contract

**This is a mirror of `Kimana_frontend/src/api/types/*`.** It is copied, not
authored here. The frontend's typed `ApiClient` interface is the backend spec
(see `docs/backend-plan.md`); this directory is how the server compiles against
the exact same shapes without a shared package existing yet.

## Rules

- **Do not edit these files to change a shape.** If the contract needs to
  change, change it in `Kimana_frontend` first, then re-copy here.
- Keep the file set in sync with the frontend's `src/api/types/`.
- Only `contract.ts` (the `ApiClient` interface) and the P1-relevant modules
  are exercised today; the rest are vendored for P2+ readiness.

## Promotion path (before P2)

Extract `Kimana_frontend/src/api/types` into a package — `@kimana/contract` —
published from the frontend repo (or a small shared repo) and added as a
dependency to both sides. Then delete this directory and import from the
package. Tracked in `docs/backend-plan.md` §00 and §06.

Source revision copied from: `Pick-MANILLA/Kimana_frontend@2645bfc`.

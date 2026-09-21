-- Real registration + login: password auth on users, DB-backed opaque
-- session tokens. Session tokens are stored hashed (never the raw value) so
-- a DB leak doesn't hand out live sessions, same spirit as password_hash.
--
-- email is nullable — existing seeded users (the demo customer) predate this
-- column — but uniqueness is enforced case-insensitively via a unique index
-- on lower(email); all lookups use lower(email) = lower($1). Postgres
-- unique-index semantics treat every NULL as distinct, so a pre-existing
-- emailless row is unaffected by this constraint.

alter table users
  add column email          text,
  add column password_hash  text;

create unique index users_email_unique_idx on users (lower(email));

create table sessions (
  id          uuid primary key default gen_random_uuid(),
  user_id     uuid not null references users (id),
  token_hash  text not null,
  created_at  timestamptz not null default now(),
  expires_at  timestamptz not null,
  revoked_at  timestamptz
);
create unique index sessions_token_hash_idx on sessions (token_hash);
create index sessions_user_idx on sessions (user_id);

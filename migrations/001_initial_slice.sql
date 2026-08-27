-- P1 slice schema: identity, onboarding, accounts, ledger, audit.
-- Money convention: every amount is <name>_minor BIGINT + a currency column.
-- ledger_entries and audit_log are append-only (enforced by trigger below).
--
-- Tables for later phases (transfers, quotes, screening, trade_documents,
-- reconciliation, ...) are intentionally absent — they arrive with their phase.

create extension if not exists "pgcrypto";

-- ---------------------------------------------------------------------------
-- identity & account
-- ---------------------------------------------------------------------------

create table users (
  id                    uuid primary key default gen_random_uuid(),
  role                  text not null check (role in ('customer', 'operator')),
  display_name          text not null,
  operator_permissions  text[] not null default '{}',
  created_at            timestamptz not null default now()
);

create table customers (
  id               uuid primary key default gen_random_uuid(),
  legal_name       text not null,
  primary_user_id  uuid not null references users (id),
  created_at       timestamptz not null default now()
);

create table accounts (
  id           uuid primary key default gen_random_uuid(),
  customer_id  uuid not null references customers (id),
  currency     text not null,
  created_at   timestamptz not null default now(),
  unique (customer_id, currency)
);

-- ---------------------------------------------------------------------------
-- onboarding
-- ---------------------------------------------------------------------------

create table onboarding_applications (
  id                 uuid primary key default gen_random_uuid(),
  customer_id        uuid not null unique references customers (id),
  status             text not null default 'draft'
                     check (status in ('draft', 'submitted', 'in_review', 'approved', 'rejected')),
  business           jsonb,
  rejection_reasons  jsonb,
  approved_summary   jsonb,
  submitted_at       timestamptz,
  reviewed_at        timestamptz,
  created_at         timestamptz not null default now(),
  updated_at         timestamptz not null default now()
);

create table onboarding_principals (
  id                    uuid primary key default gen_random_uuid(),
  application_id         uuid not null references onboarding_applications (id) on delete cascade,
  position              int not null,
  full_name             text not null,
  role                  text not null check (role in ('director', 'beneficial_owner', 'both')),
  ownership_percentage  numeric,
  date_of_birth         date,
  bvn                   text,
  nin                   text
);
create index onboarding_principals_application_idx on onboarding_principals (application_id);

create table onboarding_documents (
  id                       uuid primary key default gen_random_uuid(),
  application_id            uuid not null references onboarding_applications (id) on delete cascade,
  type                     text not null check (type in
                           ('cac_certificate', 'memart', 'proof_of_address', 'directors_id', 'board_resolution')),
  file_name                text not null,
  mime_type                text not null,
  size_bytes               bigint not null,
  status                   text not null check (status in ('pending', 'uploading', 'uploaded', 'failed')),
  upload_progress_percent  int not null default 0,
  storage_key              text,
  uploaded_at              timestamptz,
  error_message            text,
  created_at               timestamptz not null default now(),
  unique (application_id, type)
);

-- ---------------------------------------------------------------------------
-- ledger — append-only
-- ---------------------------------------------------------------------------

create table ledger_entries (
  id                     uuid primary key default gen_random_uuid(),
  account_id             uuid not null references accounts (id),
  transfer_id            uuid,
  amount_minor           bigint not null,
  currency               text not null,
  running_balance_minor  bigint not null,
  description            text not null,
  reversal_of_entry_id   uuid references ledger_entries (id),
  posted_at              timestamptz not null default now()
);
create index ledger_entries_account_idx on ledger_entries (account_id, posted_at);

-- ---------------------------------------------------------------------------
-- audit — append-only
-- ---------------------------------------------------------------------------

create table audit_log (
  id           uuid primary key default gen_random_uuid(),
  actor_id     uuid,
  actor_role   text,
  action       text not null,
  entity_type  text not null,
  entity_id    text not null,
  before       jsonb,
  after        jsonb,
  occurred_at  timestamptz not null default now()
);
create index audit_log_entity_idx on audit_log (entity_type, entity_id);
create index audit_log_occurred_idx on audit_log (occurred_at);

-- ---------------------------------------------------------------------------
-- append-only enforcement
-- ---------------------------------------------------------------------------

create or replace function forbid_row_mutation() returns trigger language plpgsql as $$
begin
  raise exception 'table % is append-only: % is not permitted', tg_table_name, tg_op;
end;
$$;

create trigger ledger_entries_append_only
  before update or delete on ledger_entries
  for each row execute function forbid_row_mutation();

create trigger audit_log_append_only
  before update or delete on audit_log
  for each row execute function forbid_row_mutation();

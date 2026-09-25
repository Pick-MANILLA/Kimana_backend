-- Collections: a customer asks to be paid, and inbound money is credited to
-- their ledger (issue #79).
--
-- Two partners take the money in:
--   * Yellow Card (NGN): every request gets its own Yellow Card "receive",
--     with a bank account the payer transfers into. provider_ref holds the
--     receive id; the receive's sequenceId is the collection id.
--   * Bridge (USD): each customer has one standing US virtual account
--     (bridge_virtual_accounts). Every deposit is credited; a deposit whose
--     memo quotes an open request's reference also marks that request paid.
--
-- collections is the receivable. Its status moves PENDING -> PAID or
-- PENDING -> CANCELLED, so unlike the ledger it is mutable. EXPIRED is not
-- stored: a PENDING row past expires_at reads as EXPIRED. Money a partner
-- confirms is always credited, so a late payment can still move a request to
-- PAID (cancelled_at then stays set as a record).
--
-- inbound_payments is the money itself: one row per payment a partner
-- confirms, keyed by the partner's id so a redelivered webhook can't credit
-- twice. Append-only, like ledger_entries; its ledger credit points back at it.

create table collections (
  id                  uuid primary key,
  -- what the payer quotes, e.g. CL-2H4F9K
  reference           text not null unique,
  customer_id         uuid not null references customers (id),
  idempotency_key     text not null,
  amount_minor        bigint not null check (amount_minor > 0),
  currency            text not null check (currency in ('NGN', 'USD')),
  payer_name          text,
  note                text,
  status              text not null default 'PENDING'
                      check (status in ('PENDING', 'PAID', 'CANCELLED')),
  provider            text not null check (provider in ('yellowcard', 'bridge')),
  provider_ref        text,
  -- the payer's instructions: bank, account number, memo
  pay_in              jsonb not null,
  expires_at          timestamptz not null,
  paid_at             timestamptz,
  cancelled_at        timestamptz,
  created_by          uuid not null references users (id),
  created_at          timestamptz not null default now(),
  updated_at          timestamptz not null default now(),
  unique (customer_id, idempotency_key),
  unique (provider, provider_ref),
  check ((status = 'PAID') = (paid_at is not null)),
  check (status <> 'CANCELLED' or cancelled_at is not null)
);
create index collections_customer_idx on collections (customer_id, created_at desc);

-- Linked by ops (`cargo run --bin link-bridge`) once the customer has passed
-- Bridge's own KYB.
create table bridge_virtual_accounts (
  customer_id           uuid primary key references customers (id),
  bridge_customer_id    text not null unique,
  virtual_account_id    text not null unique,
  deposit_instructions  jsonb not null,
  created_at            timestamptz not null default now()
);

create table inbound_payments (
  id                    uuid primary key default gen_random_uuid(),
  customer_id           uuid not null references customers (id),
  -- null for a Bridge deposit that quoted no open request
  collection_id         uuid unique references collections (id),
  provider              text not null,
  provider_payment_id   text not null,
  amount_minor          bigint not null check (amount_minor > 0),
  currency              text not null,
  payer_name            text,
  description           text,
  received_at           timestamptz not null,
  -- the partner's record as we fetched or received it
  raw                   jsonb not null,
  created_at            timestamptz not null default now(),
  unique (provider, provider_payment_id)
);
create index inbound_payments_customer_idx on inbound_payments (customer_id, received_at desc);

create trigger inbound_payments_append_only
  before update or delete on inbound_payments
  for each row execute function forbid_row_mutation();

alter table ledger_entries
  add column inbound_payment_id uuid references inbound_payments (id);

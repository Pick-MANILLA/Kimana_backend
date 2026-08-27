-- P2 · transfers + state history.
--
-- current_status is the fast-path column; transfer_state_history is the full
-- append trail. State-specific fields (fundingReference, payoutReference, hold,
-- reasonCode, ...) live in the history row's `payload` rather than as nullable
-- columns here. `position` gives deterministic ordering when several
-- transitions land in one transaction (all sharing now()).

create table transfers (
  id                    uuid primary key default gen_random_uuid(),
  reference             text not null unique,
  customer_id           uuid not null references customers (id),
  idempotency_key       text not null,
  recipient_id          uuid not null references recipients (id),
  send_currency         text not null,
  receive_currency      text not null,
  send_amount_minor     bigint not null,
  receive_amount_minor  bigint not null,
  trade_description     text,
  quote_snapshot        jsonb not null,
  current_status        text not null,
  created_at            timestamptz not null default now(),
  updated_at            timestamptz not null default now(),
  unique (customer_id, idempotency_key)
);
create index transfers_customer_idx on transfers (customer_id, created_at desc);

create table transfer_state_history (
  id           uuid primary key default gen_random_uuid(),
  transfer_id  uuid not null references transfers (id) on delete cascade,
  position     int not null,
  status       text not null,
  entered_at   timestamptz not null default clock_timestamp(),
  note         text,
  payload      jsonb,
  unique (transfer_id, position)
);
create index transfer_state_history_transfer_idx on transfer_state_history (transfer_id, position);

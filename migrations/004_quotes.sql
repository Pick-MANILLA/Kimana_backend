-- P2 · firm quotes. Immutable once issued; expires_at = issued_at + 90s.
-- createTransfer snapshots the quote onto the transfer, so a quote row is only
-- read between issue and transfer creation.

create table quotes (
  id                    uuid primary key default gen_random_uuid(),
  customer_id           uuid not null references customers (id),
  send_currency         text not null,
  receive_currency      text not null,
  rate                  double precision not null,
  fee_minor             bigint not null,
  send_amount_minor     bigint not null,
  receive_amount_minor  bigint not null,
  issued_at             timestamptz not null default now(),
  expires_at            timestamptz not null
);
create index quotes_customer_idx on quotes (customer_id);

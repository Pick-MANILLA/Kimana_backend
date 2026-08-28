-- P2 · FX rates + recipients.

-- Cache of the indicative FX feed. `rate` drifts on each read (stub provider
-- jitter), mirroring the frontend mock; a real feed would overwrite it.
create table fx_rates (
  pair                text primary key,          -- e.g. "USD/NGN"
  rate                double precision not null,
  change_percent_24h  double precision not null,
  as_of               timestamptz not null default now()
);

create table recipients (
  id                 uuid primary key default gen_random_uuid(),
  customer_id        uuid not null references customers (id),
  account_name       text not null,
  account_number     text not null,
  bank_code          text not null,
  bank_name          text not null,
  currency           text not null,
  country            text not null,              -- ISO 3166-1 alpha-2
  validation_status  text not null default 'valid'
                     check (validation_status in ('unvalidated', 'validating', 'valid', 'invalid')),
  saved_at           timestamptz not null default now()
);
create index recipients_customer_idx on recipients (customer_id);

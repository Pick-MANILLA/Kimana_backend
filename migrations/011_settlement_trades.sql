-- Settlement wallet: a customer's USDC balance and its buy/convert trades
-- (issue #77).
--
-- The SettlementVault holds one pooled USDC balance and only moves it to or
-- from partners, so a customer's USDC is a ledger account (currency 'USDC',
-- in cents like USD) and each trade moves value between it and one of the
-- customer's local-currency accounts at the indicative rate.
--
-- settlement_trades is the "coin transaction" record: one row per buy or
-- convert, with both legs in ledger_entries pointing back at it. Append-only,
-- like ledger_entries.

create table settlement_trades (
  id                  uuid primary key default gen_random_uuid(),
  reference           text not null unique,
  customer_id         uuid not null references customers (id),
  idempotency_key     text not null,
  kind                text not null check (kind in ('BUY', 'CONVERT')),
  usdc_amount_minor   bigint not null check (usdc_amount_minor > 0),
  local_currency      text not null,
  local_amount_minor  bigint not null check (local_amount_minor > 0),
  -- local-currency major units per 1 USDC (USDC is treated as USD 1:1)
  rate                double precision not null,
  rate_source         text not null check (rate_source in ('live', 'cachedProvisional')),
  created_by          uuid not null references users (id),
  created_at          timestamptz not null default now(),
  unique (customer_id, idempotency_key)
);
create index settlement_trades_customer_idx on settlement_trades (customer_id, created_at desc);

create trigger settlement_trades_append_only
  before update or delete on settlement_trades
  for each row execute function forbid_row_mutation();

alter table ledger_entries
  add column settlement_trade_id uuid references settlement_trades (id);

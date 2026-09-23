-- On-chain settlement listener (kimana_contract issue #10).
--
-- settlement_ref is the vault's key for a transfer:
-- keccak256("kimana:transfer:" || transfers.id). Events carry only the ref,
-- and a hash cannot be inverted, so it is stored when the transfer is created.
-- Transfers created before this migration have none and were never on-chain.
--
-- settlement_events is the listener's idempotency record and the audit trail
-- for the vault's per-transfer alerts (RateDivergence, ReferenceRateStale):
-- one row per applied log, keyed by (tx_hash, log_index), so a replayed block
-- is skipped. Append-only, like ledger_entries.
--
-- settlement_cursor is the last block the listener fully processed, per vault.
-- block_hash lets it detect a reorg deeper than its confirmation depth.

alter table transfers add column settlement_ref bytea unique;

create table settlement_events (
  id            uuid primary key default gen_random_uuid(),
  vault         bytea not null,
  tx_hash       bytea not null,
  log_index     bigint not null,
  block_number  bigint not null,
  block_hash    bytea not null,
  event         text not null,
  settlement_ref bytea not null,
  transfer_id   uuid not null references transfers (id) on delete cascade,
  payload       jsonb not null,
  recorded_at   timestamptz not null default now(),
  unique (tx_hash, log_index)
);
create index settlement_events_transfer_idx on settlement_events (transfer_id, block_number, log_index);

create trigger settlement_events_append_only
  before update or delete on settlement_events
  for each row execute function forbid_row_mutation();

create table settlement_cursor (
  vault         bytea primary key,
  last_block    bigint not null,
  last_block_hash bytea not null,
  updated_at    timestamptz not null default now()
);

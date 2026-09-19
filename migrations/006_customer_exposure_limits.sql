-- Per-customer exposure ceilings (Business Rule #7). NULL means "use the
-- config default" (MAX_TRANSFER_AMOUNT_MINOR / MAX_AGGREGATE_EXPOSURE_MINOR),
-- so existing customers keep the global defaults until ISSUE-02 derives
-- risk-based values.

alter table customers
  add column max_transfer_amount_minor    bigint check (max_transfer_amount_minor > 0),
  add column max_aggregate_exposure_minor bigint check (max_aggregate_exposure_minor > 0);

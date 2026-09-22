-- P2 · ties each firm quote to exactly one transfer.
--
-- create_transfer read a quote and inserted a transfer without atomically
-- marking the quote consumed, so parallel requests for the same quote_id
-- could each insert a transfer row; the settlement contract only accepts
-- the first (QuoteAlreadyUsed reverts the rest). consumed_by_transfer_id is
-- claimed with a conditional update inside the create transaction — the
-- request that updates 0 rows lost the race and is rejected with 409
-- Conflict instead of producing a transfer that can never settle.

alter table quotes
  add column consumed_by_transfer_id uuid references transfers (id);

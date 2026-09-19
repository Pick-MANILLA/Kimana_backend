-- Second FX provider stub + divergence alerting (PRD Functional Requirements
-- §B, ISSUE-03).
--
-- `fx_secondary_rates` mirrors `fx_rates`'s shape as an independently-seeded
-- second feed; a real second vendor integration replaces its jitter with a
-- live read without touching callers. `fx_rate_divergence_events` records
-- every time the two providers disagree beyond the configured threshold.

create table fx_secondary_rates (
  pair   text primary key,
  rate   double precision not null,
  as_of  timestamptz not null default now()
);

create table fx_rate_divergence_events (
  id                  uuid primary key default gen_random_uuid(),
  pair                text not null,
  provider_a          text not null,
  provider_b          text not null,
  rate_a              double precision not null,
  rate_b              double precision not null,
  divergence_percent  double precision not null,
  threshold_percent   double precision not null,
  detected_at         timestamptz not null default now()
);
create index fx_rate_divergence_events_pair_idx on fx_rate_divergence_events (pair, detected_at desc);

-- KYB check results per onboarding application. Written when `submit` runs the
-- checks; read by ops later. The customer app doesn't consume per-check detail
-- (VerificationPage animates its own checklist), only the application status.

create table kyb_checks (
  id             uuid primary key default gen_random_uuid(),
  application_id uuid not null references onboarding_applications (id) on delete cascade,
  check_key      text not null,
  passed         boolean not null,
  detail         text,
  completed_at   timestamptz not null default now(),
  unique (application_id, check_key)
);

create index kyb_checks_application_idx on kyb_checks (application_id);

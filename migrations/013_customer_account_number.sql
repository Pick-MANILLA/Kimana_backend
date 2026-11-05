-- Kimana account number: a 10-digit number that identifies a customer's
-- account inside Kimana. It is not a bank account (outside banks can't pay
-- into it); real pay-in accounts come from partners (see 012_collections).
--
-- Format: 9 random digits (the first never 0) plus a Luhn check digit, so a
-- mistyped digit or swapped pair is caught. It is assigned by the column
-- default, so every way of creating a customer (signup, the seed, tests)
-- gets one without passing it in.

create function generate_account_number() returns text
language plpgsql volatile as $$
declare
  body       text;
  candidate  text;
  total      int;
  d          int;
begin
  loop
    body := (1 + floor(random() * 9))::int::text
         || lpad(floor(random() * 100000000)::bigint::text, 8, '0');

    -- Luhn: double every second digit, starting with the rightmost of body.
    total := 0;
    for i in 0 .. 8 loop
      d := substr(body, 9 - i, 1)::int;
      if i % 2 = 0 then
        d := d * 2;
        if d > 9 then d := d - 9; end if;
      end if;
      total := total + d;
    end loop;
    candidate := body || ((10 - total % 10) % 10)::text;

    -- The unique constraint below is the real guard; this just avoids
    -- handing out a number that's already taken.
    exit when not exists (select 1 from customers where account_number = candidate);
  end loop;
  return candidate;
end
$$;

alter table customers add column account_number text;

-- Backfill one row per statement, so each new number sees the ones before it.
do $$
declare
  r record;
begin
  for r in select id from customers where account_number is null loop
    update customers set account_number = generate_account_number() where id = r.id;
  end loop;
end
$$;

alter table customers
  alter column account_number set default generate_account_number(),
  alter column account_number set not null,
  add constraint customers_account_number_key unique (account_number),
  add constraint customers_account_number_format check (account_number ~ '^[1-9][0-9]{9}$');

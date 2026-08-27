// Fixed identifiers for the single seeded demo tenant. The auth middleware
// resolves the session from DEMO_USER_ID until real session issuance lands
// (docs/backend-plan.md §01 "Auth & roles", P1-later).

export const DEMO_USER_ID = '00000000-0000-4000-8000-000000000001';
export const DEMO_CUSTOMER_ID = '00000000-0000-4000-8000-000000000002';
export const DEMO_APPLICATION_ID = '00000000-0000-4000-8000-000000000003';

export const DEMO_ACCOUNT_NGN = '00000000-0000-4000-8000-000000000010';
export const DEMO_ACCOUNT_USD = '00000000-0000-4000-8000-000000000011';
export const DEMO_ACCOUNT_EUR = '00000000-0000-4000-8000-000000000012';

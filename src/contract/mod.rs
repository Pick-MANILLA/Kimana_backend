//! Serde structs mirroring `Kimana_frontend/src/api/types/*`. The frontend's
//! typed `ApiClient` interface is the backend spec (see docs/backend-plan.md).
//! There is no shared package and no cross-language compiler check — keeping
//! these faithful to the TS contract is a review discipline.

pub mod auth;
pub mod common;
pub mod dashboard;
pub mod ledger;
pub mod onboarding;
pub mod quote;
pub mod transfer;

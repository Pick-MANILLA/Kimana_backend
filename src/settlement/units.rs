//! Integer-only helpers shared with the vault. Each one must match
//! kimana_contract exactly (`TransferRef`, `UsdcUnits`, `FxMath`), because
//! the vault recomputes the same values and rejects a quote that disagrees.
//! No `f64` anywhere: rates enter as decimal strings or scaled integers.

use alloy::primitives::{keccak256, B256, U256};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// USDC base units per cent: USDC has 6 decimals, the ledger 2.
pub const USDC_PER_CENT: u64 = 10_000;
/// Decimal places of an on-chain rate (`FxMath.RATE_DECIMALS`).
pub const RATE_DECIMALS: u32 = 8;
/// `FxMath.MAX_RATE`: 1e12 receive units per USD, 8 decimals.
pub const MAX_RATE_E8: u128 = 100_000_000_000_000_000_000;
/// `FxMath.MAX_CURRENCY_DECIMALS`.
pub const MAX_CURRENCY_DECIMALS: u8 = 18;
/// `10^(USDC_DECIMALS + RATE_DECIMALS)`, the divisor in `receiveAmount`.
const RECEIVE_DIVISOR: u128 = 100_000_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnitsError {
    #[error("{0} USDC base units is not a whole number of cents")]
    NotWholeCents(U256),
    #[error("{0} cents does not fit in the ledger's integer type")]
    CentsOverflow(U256),
    #[error("currency decimals {0} exceed the supported maximum of 18")]
    UnsupportedCurrencyDecimals(u8),
    #[error("rate must be between 1 and {MAX_RATE_E8} (8 decimals)")]
    RateOutOfRange,
    #[error("invalid rate {0:?}: expected a plain decimal with at most 8 fractional digits")]
    InvalidRate(String),
    #[error("receive amount calculation overflowed")]
    Overflow,
}

/// `TransferRef.fromTransferId`: `keccak256("kimana:transfer:" + id)`.
pub fn transfer_ref(transfer_id: &str) -> B256 {
    keccak256(format!("kimana:transfer:{transfer_id}").as_bytes())
}

/// The on-chain quote id: `keccak256` of the backend quote UUID's string form.
/// Callers pass the canonical lowercase hyphenated form (`Uuid::to_string`),
/// since a different spelling of the same UUID hashes differently.
pub fn quote_id(quote_uuid: &str) -> B256 {
    keccak256(quote_uuid.as_bytes())
}

/// `UsdcUnits.fromCents`: `cents * 10_000`.
pub fn cents_to_usdc(cents: u64) -> U256 {
    U256::from(cents) * U256::from(USDC_PER_CENT)
}

/// `UsdcUnits.toCents`: errors instead of truncating a sub-cent remainder.
pub fn usdc_to_cents(usdc: U256) -> Result<u64, UnitsError> {
    let (cents, remainder) = usdc.div_rem(U256::from(USDC_PER_CENT));
    if !remainder.is_zero() {
        return Err(UnitsError::NotWholeCents(usdc));
    }
    u64::try_from(cents).map_err(|_| UnitsError::CentsOverflow(cents))
}

/// A rate in receive-currency major units per 1 USD, scaled by 10^8
/// (`1,645.25` is `164_525_000_000`). Bounded like `lockQuote` bounds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RateE8(u128);

impl RateE8 {
    pub fn new(scaled: u128) -> Result<Self, UnitsError> {
        if scaled == 0 || scaled > MAX_RATE_E8 {
            return Err(UnitsError::RateOutOfRange);
        }
        Ok(RateE8(scaled))
    }

    pub fn get(self) -> u128 {
        self.0
    }
}

/// Parses `"1645.25"` exactly. More than 8 fractional digits is an error
/// rather than a silent rounding, so the rate locked is the rate quoted.
impl FromStr for RateE8 {
    type Err = UnitsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || UnitsError::InvalidRate(s.to_string());
        let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
        let digits_only = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
        if whole.is_empty()
            || !digits_only(whole)
            || !digits_only(frac)
            || (s.contains('.') && frac.is_empty())
            || frac.len() > RATE_DECIMALS as usize
        {
            return Err(invalid());
        }
        let padded = format!("{whole}{frac:0<8}");
        let scaled: u128 = padded.parse().map_err(|_| UnitsError::RateOutOfRange)?;
        RateE8::new(scaled)
    }
}

impl fmt::Display for RateE8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = 10u128.pow(RATE_DECIMALS);
        write!(f, "{}.{:08}", self.0 / scale, self.0 % scale)
    }
}

/// `FxMath.receiveAmount`:
/// `floor(usdc * rate_e8 * 10^decimals / 10^14)`.
pub fn receive_amount_minor(usdc: U256, rate: RateE8, decimals: u8) -> Result<U256, UnitsError> {
    if decimals > MAX_CURRENCY_DECIMALS {
        return Err(UnitsError::UnsupportedCurrencyDecimals(decimals));
    }
    let scale = U256::from(10u64).pow(U256::from(decimals));
    usdc.checked_mul(U256::from(rate.get()))
        .and_then(|v| v.checked_mul(scale))
        .map(|v| v / U256::from(RECEIVE_DIVISOR))
        .ok_or(UnitsError::Overflow)
}

// === Parity tests
// Vectors are copied from kimana_contract: test/QuoteLock.t.sol
// (`test_receiveAmount_independentVectors`, `test_receiveAmount_bounds_doNotOverflow`)
// and test/Libraries.t.sol. If one of these fails, the backend and the vault disagree.
#[cfg(test)]
mod parity_tests {
    use super::*;

    fn rate(scaled: u128) -> RateE8 {
        RateE8::new(scaled).unwrap()
    }

    #[test]
    fn receive_amount_independent_vectors() {
        let cases: [(u64, u128, u8, U256); 5] = [
            (1_000_000, 164_525_000_000, 2, U256::from(164_525u64)),
            (1, 164_525_000_000, 2, U256::ZERO),
            (10_000, 164_525_000_000, 2, U256::from(1645u64)),
            (2_500_000_000, 1_210_000_000, 2, U256::from(3_025_000u64)),
            (
                1_000_000,
                100_000_000,
                18,
                U256::from(10u64).pow(U256::from(18)),
            ),
        ];
        for (usdc, r, decimals, expected) in cases {
            assert_eq!(
                receive_amount_minor(U256::from(usdc), rate(r), decimals).unwrap(),
                expected,
                "usdc={usdc} rate={r} decimals={decimals}"
            );
        }
    }

    #[test]
    fn receive_amount_bounds_do_not_overflow() {
        let e18 = U256::from(10u64).pow(U256::from(18));
        let expected = e18 * U256::from(MAX_RATE_E8) * e18 / U256::from(RECEIVE_DIVISOR);
        assert_eq!(
            receive_amount_minor(e18, rate(MAX_RATE_E8), 18).unwrap(),
            expected
        );
    }

    #[test]
    fn receive_amount_rejects_too_many_decimals() {
        assert_eq!(
            receive_amount_minor(U256::from(1_000_000u64), rate(100_000_000), 19),
            Err(UnitsError::UnsupportedCurrencyDecimals(19))
        );
    }

    #[test]
    fn e2e_blocked_quote_vector() {
        // script/e2e-local.sh: RECV3 = floor(AMT3 * RATE3 * 100 / 1e14)
        assert_eq!(
            receive_amount_minor(U256::from(1_000_000_000u64), rate(174_396_500_000), 2).unwrap(),
            U256::from(174_396_500u64)
        );
    }

    #[test]
    fn from_cents_known_values() {
        assert_eq!(cents_to_usdc(1), U256::from(10_000u64));
        assert_eq!(cents_to_usdc(100), U256::from(1_000_000u64));
        assert_eq!(cents_to_usdc(4_500_000), U256::from(45_000_000_000u64));
    }

    #[test]
    fn to_cents_rejects_sub_cent_amounts() {
        assert_eq!(
            usdc_to_cents(U256::from(10_001u64)),
            Err(UnitsError::NotWholeCents(U256::from(10_001u64)))
        );
        assert_eq!(usdc_to_cents(U256::from(45_000_000_000u64)), Ok(4_500_000));
    }

    #[test]
    fn cents_round_trip() {
        for cents in [0, 1, 99, 100, 150_000, 4_500_000, u64::MAX] {
            assert_eq!(usdc_to_cents(cents_to_usdc(cents)), Ok(cents));
        }
    }

    #[test]
    fn transfer_ref_matches_contract_formula() {
        assert_eq!(
            transfer_ref("txn_0001"),
            keccak256("kimana:transfer:txn_0001".as_bytes())
        );
        assert_ne!(transfer_ref("txn_0001"), transfer_ref("txn_0002"));
    }

    #[test]
    fn transfer_ref_matches_cast_keccak() {
        // `cast keccak "kimana:transfer:e2e_txn_001"`, the ref LocalE2E.s.sol settles.
        assert_eq!(
            transfer_ref("e2e_txn_001"),
            "0xf2e95e83f190b366a89eda0a7186786eea99cf877681a8bd88b09ffb1b46b90b"
                .parse::<B256>()
                .unwrap()
        );
    }

    #[test]
    fn quote_id_hashes_the_uuid_string() {
        // `cast keccak "3f2504e0-4f89-41d3-9a0c-0305e82c3301"`
        assert_eq!(
            quote_id("3f2504e0-4f89-41d3-9a0c-0305e82c3301"),
            "0x9c59762b2040c61ed687aaf4422e0ba058bb89d0397349c59cd036eb2315907f"
                .parse::<B256>()
                .unwrap()
        );
    }

    #[test]
    fn rate_parses_exactly() {
        assert_eq!("1645.25".parse::<RateE8>().unwrap().get(), 164_525_000_000);
        assert_eq!("12.10".parse::<RateE8>().unwrap().get(), 1_210_000_000);
        assert_eq!("1".parse::<RateE8>().unwrap().get(), 100_000_000);
        assert_eq!("0.00000001".parse::<RateE8>().unwrap().get(), 1);
        assert_eq!(
            "1000000000000".parse::<RateE8>().unwrap().get(),
            MAX_RATE_E8
        );
        assert_eq!(rate(164_525_000_000).to_string(), "1645.25000000");
    }

    #[test]
    fn rate_rejects_bad_input() {
        for bad in ["", ".5", "1.", "1.123456789", "-1", "1e3", "1,645.25", " 1"] {
            assert!(bad.parse::<RateE8>().is_err(), "{bad:?} should not parse");
        }
        assert_eq!("0".parse::<RateE8>(), Err(UnitsError::RateOutOfRange));
        assert_eq!(
            "1000000000000.00000001".parse::<RateE8>(),
            Err(UnitsError::RateOutOfRange)
        );
    }
}

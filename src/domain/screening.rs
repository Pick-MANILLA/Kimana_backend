//! Stub compliance screening. Every transfer is screened against documented,
//! testable trigger conditions — so the hold path is exercisable:
//!
//! - `watchlist_country`: the recipient's country is on a sanctioned-country
//!   list.
//! - `amount_threshold`: the send amount is at or above the configured
//!   manual-review threshold.
//!
//! A customer-risk-rating trigger belongs here too, but `customers` has no
//! risk-rating column yet — wiring a trigger to a field nothing ever sets
//! would be theater, not a control. Add it once that data exists.
//!
//! Swap this module for a real provider (sanctions/PEP list screening,
//! transaction monitoring) when the ops back-office work lands.

use crate::contract::common::CurrencyCode;

/// ISO 3166-1 alpha-2 codes under comprehensive sanctions programs.
pub const WATCHLISTED_COUNTRIES: [&str; 5] = ["IR", "KP", "SY", "CU", "RU"];

pub struct ScreeningInput<'a> {
    pub recipient_country: &'a str,
    pub send_amount_minor: i64,
    pub send_currency: CurrencyCode,
}

pub struct ScreeningOutcome {
    pub hold: bool,
    pub hold_reason: Option<String>,
}

pub fn screen(input: ScreeningInput, amount_threshold_minor: i64) -> ScreeningOutcome {
    let country = input.recipient_country.trim().to_uppercase();
    if WATCHLISTED_COUNTRIES.contains(&country.as_str()) {
        return ScreeningOutcome {
            hold: true,
            hold_reason: Some(format!(
                "Recipient country {country} is on the sanctioned-country watchlist."
            )),
        };
    }

    if input.send_amount_minor >= amount_threshold_minor {
        return ScreeningOutcome {
            hold: true,
            hold_reason: Some(format!(
                "Send amount {} {} is at or above the compliance review threshold.",
                input.send_amount_minor,
                input.send_currency.as_str()
            )),
        };
    }

    ScreeningOutcome {
        hold: false,
        hold_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd() -> CurrencyCode {
        CurrencyCode::parse("USD").unwrap()
    }

    #[test]
    fn watchlisted_country_holds_regardless_of_amount() {
        let outcome = screen(
            ScreeningInput {
                recipient_country: "ir",
                send_amount_minor: 100,
                send_currency: usd(),
            },
            5_000_000,
        );
        assert!(outcome.hold);
        assert!(outcome.hold_reason.unwrap().contains("IR"));
    }

    #[test]
    fn amount_at_or_above_threshold_holds() {
        let outcome = screen(
            ScreeningInput {
                recipient_country: "NL",
                send_amount_minor: 5_000_000,
                send_currency: usd(),
            },
            5_000_000,
        );
        assert!(outcome.hold);
    }

    #[test]
    fn clean_transfer_passes() {
        let outcome = screen(
            ScreeningInput {
                recipient_country: "NL",
                send_amount_minor: 4_500_000,
                send_currency: usd(),
            },
            5_000_000,
        );
        assert!(!outcome.hold);
        assert!(outcome.hold_reason.is_none());
    }
}

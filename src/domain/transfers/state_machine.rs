//! Explicit transition table. Every state change — engine or ops — goes
//! through `assert_transition`.

use crate::contract::transfer::TransferStatus::{self, *};
use crate::error::ApiError;

pub fn allowed(from: TransferStatus) -> &'static [TransferStatus] {
    match from {
        Created => &[Quoted, Expired],
        Quoted => &[Screened, Expired],
        Screened => &[AwaitingFunds, Rejected, Expired],
        AwaitingFunds => &[Funded, Rejected, Expired],
        Funded => &[Settling, Rejected],
        Settling => &[Settled, Rejected],
        Settled => &[PayingOut],
        PayingOut => &[Completed],
        Completed => &[Reversing],
        Rejected => &[],
        Expired => &[],
        Reversing => &[Reversed],
        Reversed => &[],
    }
}

/// The single forward step on the happy path, or None at a waypoint/terminal.
pub fn forward_step(from: TransferStatus) -> Option<TransferStatus> {
    Some(match from {
        Created => Quoted,
        Quoted => Screened,
        Screened => AwaitingFunds,
        AwaitingFunds => Funded,
        Funded => Settling,
        Settling => Settled,
        Settled => PayingOut,
        PayingOut => Completed,
        _ => return None,
    })
}

pub fn assert_transition(from: TransferStatus, to: TransferStatus) -> Result<(), ApiError> {
    if allowed(from).contains(&to) {
        Ok(())
    } else {
        Err(ApiError::conflict(format!(
            "Transfer cannot move from {} to {}.",
            from.as_str(),
            to.as_str()
        )))
    }
}

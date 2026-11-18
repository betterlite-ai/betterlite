use betterlite_auth::user_roles::UserRole;
use rust_decimal::Decimal;

use crate::key_delegation::caller::Caller;
use crate::key_delegation::error::DelegationError;
use crate::management_guards::amounts::{Amount, describe_amount};

// LiteLLM types a window's max_budget as a float, so NaN and both infinities fail before any admin bypass.
pub fn check_budget_ceilings(
    budget_limits: Option<&[Amount]>,
    delegation_ceiling: Option<Amount>,
    caller: Caller,
    has_team: bool,
) -> Result<(), DelegationError> {
    let Some(windows) = budget_limits.filter(|windows| !windows.is_empty()) else {
        return Ok(());
    };
    if let Some(amount) = windows
        .iter()
        .find(|window| !matches!(window, Amount::Finite(_)))
    {
        return Err(DelegationError::NonFiniteBudget {
            amount: describe_amount(Some(*amount)),
        });
    }
    if caller.role == Some(UserRole::ProxyAdmin) || caller.is_ui_session_team_key {
        return Ok(());
    }
    if caller.is_session_token && !has_team {
        return Err(DelegationError::SessionTokenWithoutTeam);
    }
    let Some(ceiling) = delegation_ceiling else {
        return Ok(());
    };
    let over = windows.iter().find(|amount| match amount {
        Amount::Finite(window) => exceeds_window(*window, ceiling),
        Amount::Nan | Amount::PositiveInfinity | Amount::NegativeInfinity => false,
    });
    let Some(amount) = over else {
        return Ok(());
    };
    Err(DelegationError::AboveCeiling {
        amount: describe_amount(Some(*amount)),
        ceiling: describe_amount(Some(ceiling)),
    })
}

// Python compares floats: a NaN or +inf ceiling exceeds no window, and a -inf one exceeds every finite window.
fn exceeds_window(window: Decimal, ceiling: Amount) -> bool {
    match ceiling {
        Amount::Finite(limit) => window > limit,
        Amount::Nan | Amount::PositiveInfinity => false,
        Amount::NegativeInfinity => true,
    }
}

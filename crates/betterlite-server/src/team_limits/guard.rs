use betterlite_auth::user_roles::UserRole;
use rust_decimal::Decimal;

use crate::key_fields::values::Amount;
use crate::team_limits::defaults::Caller;
use crate::team_limits::field_supplied;
use crate::team_request_contracts::update::UpdateTeamRequest;

// the guarded write only lands while the stored max_budget still equals the value this verdict saw
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxBudgetGuard {
    pub expected: Option<Amount>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BudgetUpdateError {
    #[error(
        "Only a proxy admin can remove a team's max_budget. Team's current max_budget={existing}."
    )]
    RemoveDenied { existing: Decimal },
    #[error(
        "Only a proxy admin can raise a team's max_budget. Team's current max_budget={existing}, requested={requested}."
    )]
    RaiseDenied {
        existing: Decimal,
        requested: Decimal,
    },
}

pub fn budget_update_authority(
    request: &UpdateTeamRequest,
    caller: &Caller,
    existing_team_max_budget: Option<Amount>,
) -> Result<Option<MaxBudgetGuard>, BudgetUpdateError> {
    if caller.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(None);
    }
    let explicitly_set = field_supplied(request, "max_budget");
    let guard = explicitly_set.then_some(MaxBudgetGuard {
        expected: existing_team_max_budget,
    });
    let Some(existing) = existing_team_max_budget else {
        return Ok(guard);
    };
    let requested = request.fields.max_budget;
    if explicitly_set && requested.is_none() {
        return Err(BudgetUpdateError::RemoveDenied {
            existing: existing.0,
        });
    }
    if let Some(amount) = requested
        && amount.0 > existing.0
    {
        return Err(BudgetUpdateError::RaiseDenied {
            existing: existing.0,
            requested: amount.0,
        });
    }
    Ok(guard)
}

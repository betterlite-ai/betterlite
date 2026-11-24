use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::expiry::{TempBudgetError, parse_temp_budget_expiry};
use super::money::Money;

// the copy _update_key_budget_with_temp_budget_increase returns: the request's effective maximum, and whether the increase produced it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TempBudgetUpdate {
    pub max_budget: Option<Money>,
    pub applied: bool,
}

// _get_temp_budget_increase: both metadata fields are required, and only a future expiry applies
pub fn get_temp_budget_increase(
    auth: &UserAPIKeyAuth,
    now: DateTime<Utc>,
) -> Result<Option<Money>, TempBudgetError> {
    let metadata = &auth.view.token.metadata.0;
    let Some(increase) = metadata.get("temp_budget_increase") else {
        return Ok(None);
    };
    let Some(expiry) = metadata.get("temp_budget_expiry") else {
        return Ok(None);
    };
    let JsonData::String(expiry_text) = expiry else {
        return Err(TempBudgetError::ExpiryInvalid);
    };
    if parse_temp_budget_expiry(expiry_text)? <= now {
        return Ok(None);
    }
    // the metadata value is not a declared field: only a JSON number is the increase LiteLLM's addition accepts
    match increase {
        JsonData::Number(number) => Money::from_text(&number.to_string())
            .ok_or(TempBudgetError::IncreaseInvalid)
            .map(Some),
        JsonData::Null => Ok(None),
        JsonData::Bool(_) | JsonData::String(_) | JsonData::Array(_) | JsonData::Object(_) => {
            Err(TempBudgetError::IncreaseInvalid)
        }
    }
}

// _update_key_budget_with_temp_budget_increase: no maximum skips the parse, and a zero increase leaves the original maximum
pub fn apply_temp_budget_increase(
    auth: &UserAPIKeyAuth,
    now: DateTime<Utc>,
) -> Result<TempBudgetUpdate, TempBudgetError> {
    let Some(max_budget) = auth.view.token.max_budget else {
        return Ok(TempBudgetUpdate {
            max_budget: None,
            applied: false,
        });
    };
    let original = Some(Money::Finite(max_budget));
    match get_temp_budget_increase(auth, now)? {
        Some(increase) if !matches!(increase, Money::Finite(value) if value.is_zero()) => {
            Ok(TempBudgetUpdate {
                max_budget: Some(Money::Finite(max_budget).plus(increase)),
                applied: true,
            })
        }
        Some(_) | None => Ok(TempBudgetUpdate {
            max_budget: original,
            applied: false,
        }),
    }
}

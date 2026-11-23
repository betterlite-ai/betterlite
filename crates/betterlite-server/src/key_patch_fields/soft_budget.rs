use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::management_guards::amounts::{Amount, describe_amount};

use super::Row;
use super::error::KeyPatchError;

pub const SOFT_BUDGET: &str = "soft_budget";
pub const MAX_BUDGET: &str = "max_budget";

// _validate_soft_budget_value and _validate_max_budget: None and zero pass; non-finite or below-zero is the 400 (so -0.0 passes).
pub fn validate_budget(field: &'static str, amount: Option<Amount>) -> Result<(), KeyPatchError> {
    let rejected = match amount {
        Some(Amount::Nan | Amount::PositiveInfinity | Amount::NegativeInfinity) => amount,
        Some(Amount::Finite(decimal)) if decimal.is_sign_negative() && !decimal.is_zero() => amount,
        Some(Amount::Finite(_)) | None => None,
    };
    let Some(rejected_amount) = rejected else {
        return Ok(());
    };
    Err(KeyPatchError::NonFiniteBudget {
        field,
        received: describe_amount(Some(rejected_amount)),
    })
}

// The two budget operations `_update_key_soft_budget` drives; production runs them inside the key transaction.
#[async_trait]
pub trait SoftBudgetStore {
    async fn update_soft_budget(
        &self,
        budget_id: &str,
        soft_budget: Option<Decimal>,
        updated_by: &str,
    ) -> Result<(), KeyPatchError>;

    async fn create_soft_budget(
        &self,
        soft_budget: Decimal,
        created_by: &str,
    ) -> Result<String, KeyPatchError>;
}

fn storable(amount: Amount) -> Result<Decimal, KeyPatchError> {
    match amount {
        Amount::Finite(decimal) => Ok(decimal),
        Amount::Nan | Amount::PositiveInfinity | Amount::NegativeInfinity => {
            Err(KeyPatchError::NonFiniteBudget {
                field: SOFT_BUDGET,
                received: describe_amount(Some(amount)),
            })
        }
    }
}

// _update_key_soft_budget: an existing budget row takes the new soft budget, otherwise a nonnull one creates it.
pub async fn update_key_soft_budget<S>(
    store: &S,
    existing_budget_id: Option<&str>,
    soft_budget: Option<Amount>,
    changed_by: &str,
) -> Result<Option<String>, KeyPatchError>
where
    S: SoftBudgetStore + Sync,
{
    if let Some(budget_id) = existing_budget_id {
        let amount = soft_budget.map(storable).transpose()?;
        store
            .update_soft_budget(budget_id, amount, changed_by)
            .await?;
        return Ok(Some(budget_id.to_owned()));
    }
    let Some(amount) = soft_budget else {
        return Ok(None);
    };
    let created = store
        .create_soft_budget(storable(amount)?, changed_by)
        .await?;
    Ok(Some(created))
}

// _apply_soft_budget_update: the soft_budget request field never reaches the key row, and a new budget is linked.
pub async fn apply_soft_budget_update<S>(
    store: &S,
    soft_budget: Option<Amount>,
    values: Row,
    existing_budget_id: Option<&str>,
    changed_by: &str,
) -> Result<Row, KeyPatchError>
where
    S: SoftBudgetStore + Sync,
{
    let mut remaining = values;
    remaining.remove(SOFT_BUDGET);
    let updated =
        update_key_soft_budget(store, existing_budget_id, soft_budget, changed_by).await?;
    if let (Some(budget_id), None) = (updated, existing_budget_id) {
        remaining.insert(String::from("budget_id"), JsonData::String(budget_id));
    }
    Ok(remaining)
}

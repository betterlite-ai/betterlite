use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use rust_decimal::Decimal;

use super::error::CallError;

// litellm.budget_exceeded_status_code
pub const BUDGET_EXCEEDED_STATUS: u16 = 422;

// litellm.max_budget and litellm._current_cost; both are Decimal so the boundary compares exactly
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BudgetState {
    pub max_budget: Option<Decimal>,
    pub current_cost: Decimal,
}

impl BudgetState {
    pub fn check(&self) -> Result<(), CallError> {
        let Some(max_budget) = self.max_budget else {
            return Ok(());
        };
        if self.current_cost > max_budget {
            return Err(CallError::from(ErrorClass::BudgetExceeded(Box::new(
                BudgetExceededDetails {
                    current_cost: self.current_cost,
                    max_budget,
                    message: None,
                    provider: String::new(),
                    entity_type: None,
                    entity_id: None,
                    status_code: BUDGET_EXCEEDED_STATUS,
                },
            ))));
        }
        Ok(())
    }
}

mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;

use super::ModelBudgetLimiter;
use super::cache::ModelBudgetCache;
use super::enforcement::{BudgetSubject, subject_within_model_budget};

pub struct FallbackQuery<'input> {
    pub entity_id: &'input str,
    pub key_alias: Option<&'input str>,
    pub model_max_budget: Option<&'input BTreeMap<String, JsonData>>,
    pub budget_fallbacks: Option<&'input BTreeMap<String, Vec<String>>>,
    pub model: &'input str,
}

// the first model in the key's chain that passes the key's own model budget, or None
pub async fn fallback_model_within_budget<C>(
    limiter: &ModelBudgetLimiter<C>,
    query: FallbackQuery<'_>,
) -> Result<Option<String>, ErrorClass>
where
    C: ModelBudgetCache,
{
    let Some(candidates) = query
        .budget_fallbacks
        .and_then(|fallbacks| fallbacks.get(query.model))
    else {
        return Ok(None);
    };
    for candidate in candidates {
        let subject = BudgetSubject::Key {
            entity_id: query.entity_id,
            key_alias: query.key_alias,
        };
        let within =
            subject_within_model_budget(limiter, &subject, query.model_max_budget, candidate).await;
        match within {
            Ok(()) => return Ok(Some(candidate.clone())),
            Err(ErrorClass::BudgetExceeded(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

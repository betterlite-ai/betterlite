use std::collections::BTreeMap;

use betterlite_auth::auth_tag_budgets::TagBudgetError;
use betterlite_auth::auth_tag_budgets::cache::TagBudgetCache;
use betterlite_auth::auth_tag_budgets::rows::BudgetRows;
use betterlite_auth::auth_tag_budgets::tags::get_tag_objects_batch;
use betterlite_db::managed_resource_rows::tag::TagTable;

use crate::limiter_records::{TagRateLimit, TagRateLimitResolver};

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum TagResolutionError {
    #[error(transparent)]
    TagBudget(#[from] TagBudgetError),
}

// LiteLLM's resolve_tag_rate_limits_from_db: keep the tags whose budget table carries at least one ceiling
pub async fn resolve_tag_rate_limits(
    tag_names: &[String],
    database: bool,
    cache: &dyn TagBudgetCache,
    rows: &dyn BudgetRows<TagTable>,
) -> Result<BTreeMap<String, TagRateLimit>, TagResolutionError> {
    let tag_objects = get_tag_objects_batch(tag_names, database, cache, rows).await?;
    let mut limits = BTreeMap::new();
    for (tag_name, tag_object) in tag_objects {
        let Some(budget) = tag_object.litellm_budget_table else {
            continue;
        };
        if budget.rpm_limit.is_none() && budget.tpm_limit.is_none() {
            continue;
        }
        limits.insert(
            tag_name,
            TagRateLimit {
                rpm_limit: budget.rpm_limit,
                tpm_limit: budget.tpm_limit,
            },
        );
    }
    Ok(limits)
}

pub struct DbTagRateLimitResolver<'lookup> {
    pub database: bool,
    pub cache: &'lookup dyn TagBudgetCache,
    pub rows: &'lookup dyn BudgetRows<TagTable>,
}

// The pre-call hook cannot fail on a tag read: an unavailable lookup leaves the tag descriptors absent
#[async_trait::async_trait]
impl TagRateLimitResolver for DbTagRateLimitResolver<'_> {
    async fn resolve(&self, tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        match resolve_tag_rate_limits(tag_names, self.database, self.cache, self.rows).await {
            Ok(limits) => limits,
            Err(error) => {
                tracing::debug!(error = %error, "Error resolving tag rate limits from database");
                BTreeMap::new()
            }
        }
    }
}

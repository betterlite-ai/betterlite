use betterlite_logging::prometheus::budget_lookups::key_budget::{
    KeyBudgetRequest,
    set_key_budget_metrics_after_api_request as write_key_budget, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus::budget_lookups::snapshots;
use chrono::Utc;

use crate::prometheus_remaining_budget::lookups::{KeyBudgetLookup, LookupError};

use super::{BudgetLookupPort, budget_error};

impl BudgetLookupPort {
    pub(super) async fn key_after_api_request(
        &self,
        lookup: KeyBudgetLookup<'_>,
    ) -> Result<(), LookupError> {
        let carried = lookup.carried.map(|snapshot| snapshots::KeyBudgetSnapshot {
            budget_reset_at: snapshot.budget_reset_at,
        });
        let request = KeyBudgetRequest {
            user_api_key: lookup.user_api_key,
            user_api_key_alias: lookup.user_api_key_alias,
            response_cost: lookup.response_cost,
            key_max_budget: lookup.key_max_budget,
            key_spend: lookup.key_spend,
            carried: carried.as_ref(),
        };
        write_key_budget(
            self.lookups.as_ref(),
            &self.gauges.key,
            self.source(),
            Utc::now(),
            &request,
        )
        .await
        .map_err(budget_error)
    }
}

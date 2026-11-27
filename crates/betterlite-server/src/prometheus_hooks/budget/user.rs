use betterlite_logging::prometheus::budget_lookups::snapshots;
use betterlite_logging::prometheus::budget_lookups::user_budget::{
    UserBudgetRequest,
    set_user_budget_metrics_after_api_request as write_user_budget, // ast-grep-ignore: no-long-names
};
use chrono::Utc;

use crate::prometheus_remaining_budget::lookups::{LookupError, UserBudgetLookup};

use super::{BudgetLookupPort, budget_error};

impl BudgetLookupPort {
    pub(super) async fn user_after_api_request(
        &self,
        lookup: UserBudgetLookup<'_>,
    ) -> Result<(), LookupError> {
        let carried = match lookup.carried {
            Some(snapshot) => Some(snapshots::UserBudgetSnapshot {
                budget_reset_at: snapshot.budget_reset_at,
                max_budget: snapshot.max_budget,
                user_alias: snapshot.user_alias,
            }),
            None => None,
        };
        let request = UserBudgetRequest {
            user_id: lookup.user_id,
            user_spend: lookup.user_spend,
            user_max_budget: lookup.user_max_budget,
            response_cost: lookup.response_cost,
            carried: carried.as_ref(),
            user_email: lookup.user_email,
        };
        write_user_budget(
            self.lookups.as_ref(),
            &self.gauges.user,
            self.source(),
            Utc::now(),
            &request,
        )
        .await
        .map_err(budget_error)
    }
}

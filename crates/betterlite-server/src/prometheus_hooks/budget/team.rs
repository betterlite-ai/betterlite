use betterlite_logging::prometheus::budget_lookups::snapshots;
use betterlite_logging::prometheus::budget_lookups::team_budget::{
    TeamBudgetRequest,
    set_team_budget_metrics_after_api_request as write_team_budget, // ast-grep-ignore: no-long-names
};
use chrono::Utc;

use crate::prometheus_remaining_budget::lookups::{LookupError, TeamBudgetLookup};

use super::{BudgetLookupPort, budget_error};

impl BudgetLookupPort {
    pub(super) async fn team_after_api_request(
        &self,
        lookup: TeamBudgetLookup<'_>,
    ) -> Result<(), LookupError> {
        let carried = lookup
            .carried
            .map(|snapshot| snapshots::TeamBudgetSnapshot {
                budget_reset_at: snapshot.budget_reset_at,
                max_budget: snapshot.max_budget,
            });
        write_team_budget(
            self.lookups.as_ref(),
            &self.gauges.team,
            self.source(),
            Utc::now(),
            &TeamBudgetRequest {
                user_api_team: lookup.user_api_team,
                user_api_team_alias: lookup.user_api_team_alias,
                team_spend: lookup.team_spend,
                team_max_budget: lookup.team_max_budget,
                response_cost: lookup.response_cost,
                carried: carried.as_ref(),
            },
        )
        .await
        .map_err(budget_error)
    }
}

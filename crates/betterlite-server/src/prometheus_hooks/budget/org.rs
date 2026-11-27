use betterlite_logging::prometheus::budget_lookups::org_budget::{
    OrgBudgetRequest,
    set_org_budget_metrics_after_api_request as write_org_budget, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus::budget_lookups::snapshots;
use chrono::Utc;

use crate::prometheus_remaining_budget::lookups::{LookupError, OrgBudgetLookup};

use super::{BudgetLookupPort, budget_error};

impl BudgetLookupPort {
    pub(super) async fn org_after_api_request(
        &self,
        lookup: OrgBudgetLookup<'_>,
    ) -> Result<(), LookupError> {
        let carried = lookup.carried.map(|snapshot| snapshots::OrgBudgetSnapshot {
            spend: snapshot.spend,
            max_budget: snapshot.max_budget,
        });
        let request = OrgBudgetRequest {
            org_id: lookup.org_id,
            response_cost: lookup.response_cost,
            carried: carried.as_ref(),
            org_alias: lookup.org_alias,
        };
        write_org_budget(
            self.lookups.as_ref(),
            &self.gauges.org,
            self.source(),
            Utc::now(),
            &request,
        )
        .await
        .map_err(budget_error)
    }
}

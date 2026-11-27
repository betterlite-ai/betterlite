use std::time::Instant;

use betterlite_logging::prometheus::budget_gauges::customer::{
    BudgetGaugeClock, CustomerGaugeContext,
};
use betterlite_logging::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest,
    set_customer_budget_metrics_after_api_request as write_customer_budget, // ast-grep-ignore: no-long-names
};
use chrono::Utc;

use crate::prometheus_remaining_budget::lookups::{CustomerBudgetLookup, LookupError};

use super::{BudgetLookupPort, budget_error};

impl BudgetLookupPort {
    pub(super) async fn customer_after_api_request(
        &self,
        lookup: CustomerBudgetLookup<'_>,
    ) -> Result<(), LookupError> {
        let request = CustomerBudgetRequest {
            end_user_id: lookup.end_user_id,
            response_cost: lookup.response_cost,
        };
        let context = CustomerGaugeContext {
            source: self.source(),
            clock: BudgetGaugeClock {
                reset: Utc::now(),
                monotonic: Instant::now(),
            },
            tracker: &self.collectors.bounded_series_tracker,
            limits: &self.limits,
        };
        write_customer_budget(
            self.lookups.as_ref(),
            &self.gauges.customer,
            context,
            self.end_user,
            &request,
        )
        .await
        .map_err(budget_error)
    }
}

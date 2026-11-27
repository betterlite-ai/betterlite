use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::BudgetGaugeError;
use betterlite_logging::prometheus::budget_gauges::gauge::BudgetGauges;
use betterlite_logging::prometheus::budget_lookups::lookups::BudgetLookups;
use betterlite_logging::prometheus::budget_lookups::org_budget::{
    OrgBudgetRequest,
    set_org_budget_metrics_after_api_request as write_org_budget, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus_exporter::{Handle, PrometheusCollectors};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::prometheus_remaining_budget::lookups::{
    CustomerBudgetLookup, KeyBudgetLookup, LookupError, OrgBudgetLookup, RequestBudgetLookups,
    TeamBudgetLookup, UserBudgetLookup,
};

use super::gauges::budget_gauges;

mod customer;
mod key;
mod org;
mod team;
mod user;

// LiteLLM's PrometheusLogger owns the gauges; this port wraps B3883's cache and database reads and writes them through
pub struct BudgetLookupPort {
    lookups: Arc<dyn BudgetLookups>,
    collectors: Arc<PrometheusCollectors>,
    pub(crate) gauges: BudgetGauges<Handle>,
    settings: PrometheusLabelSettings,
    limits: SeriesLimits,
    end_user: EndUserCostTrackingSettings,
}

impl BudgetLookupPort {
    #[must_use]
    pub fn new(
        lookups: Arc<dyn BudgetLookups>,
        collectors: Arc<PrometheusCollectors>,
        settings: PrometheusLabelSettings,
        limits: SeriesLimits,
        end_user: EndUserCostTrackingSettings,
    ) -> Self {
        let gauges = budget_gauges(collectors.as_ref());
        Self {
            lookups,
            collectors,
            gauges,
            settings,
            limits,
            end_user,
        }
    }

    // the failure event writes the org gauge with a zero cost and no carried snapshot, outside the fan-out
    pub(crate) async fn org_after_failure(&self, org_id: Option<&str>) -> Result<(), LookupError> {
        let request = OrgBudgetRequest {
            org_id,
            response_cost: Decimal::ZERO,
            carried: None,
            org_alias: None,
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

    const fn source(&self) -> LabelSource<'_> {
        LabelSource::Values {
            settings: &self.settings,
            end_user_settings: self.end_user,
        }
    }
}

#[async_trait]
impl RequestBudgetLookups for Arc<BudgetLookupPort> {
    async fn key(&self, lookup: KeyBudgetLookup<'_>) -> Result<(), LookupError> {
        self.key_after_api_request(lookup).await
    }

    async fn team(&self, lookup: TeamBudgetLookup<'_>) -> Result<(), LookupError> {
        self.team_after_api_request(lookup).await
    }

    async fn user(&self, lookup: UserBudgetLookup<'_>) -> Result<(), LookupError> {
        self.user_after_api_request(lookup).await
    }

    async fn org(&self, lookup: OrgBudgetLookup<'_>) -> Result<(), LookupError> {
        self.org_after_api_request(lookup).await
    }

    async fn customer(&self, lookup: CustomerBudgetLookup<'_>) -> Result<(), LookupError> {
        self.customer_after_api_request(lookup).await
    }
}

fn budget_error<MetricError>(error: BudgetGaugeError<MetricError>) -> LookupError
where
    MetricError: std::error::Error + 'static,
{
    LookupError {
        message: error.to_string(),
    }
}

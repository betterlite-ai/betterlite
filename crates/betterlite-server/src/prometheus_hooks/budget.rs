use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::BudgetGaugeError;
use betterlite_logging::prometheus::budget_gauges::customer::{
    BudgetGaugeClock, CustomerGaugeContext,
};
use betterlite_logging::prometheus::budget_gauges::gauge::BudgetGauges;
use betterlite_logging::prometheus::budget_lookups::customer_budget::{
    CustomerBudgetRequest,
    set_customer_budget_metrics_after_api_request as write_customer_budget, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus::budget_lookups::key_budget::{
    KeyBudgetRequest,
    set_key_budget_metrics_after_api_request as write_key_budget, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus::budget_lookups::lookups::BudgetLookups;
use betterlite_logging::prometheus::budget_lookups::org_budget::{
    OrgBudgetRequest,
    set_org_budget_metrics_after_api_request as write_org_budget, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus::budget_lookups::snapshots;
use betterlite_logging::prometheus::budget_lookups::team_budget::{
    TeamBudgetRequest,
    set_team_budget_metrics_after_api_request as write_team_budget, // ast-grep-ignore: no-long-names
};
use betterlite_logging::prometheus::budget_lookups::user_budget::{
    UserBudgetRequest,
    set_user_budget_metrics_after_api_request as write_user_budget, // ast-grep-ignore: no-long-names
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

// LiteLLM's PrometheusLogger owns the gauges; this port wraps B3883's cache and database reads and writes them through
pub struct BudgetLookupPort {
    lookups: Arc<dyn BudgetLookups>,
    collectors: Arc<PrometheusCollectors>,
    pub gauges: BudgetGauges<Handle>,
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

    async fn team(&self, lookup: TeamBudgetLookup<'_>) -> Result<(), LookupError> {
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

    async fn user(&self, lookup: UserBudgetLookup<'_>) -> Result<(), LookupError> {
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

    async fn org(&self, lookup: OrgBudgetLookup<'_>) -> Result<(), LookupError> {
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

    async fn customer(&self, lookup: CustomerBudgetLookup<'_>) -> Result<(), LookupError> {
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

fn budget_error<MetricError>(error: BudgetGaugeError<MetricError>) -> LookupError
where
    MetricError: std::error::Error + 'static,
{
    LookupError {
        message: error.to_string(),
    }
}

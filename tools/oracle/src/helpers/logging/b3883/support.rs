use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::Future;
use std::time::Instant;

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::customer::{
    BudgetGaugeClock, CustomerGaugeContext,
};
use betterlite_logging::prometheus::budget_gauges::gauge::{EntityBudgetGauges, GaugeField};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prometheus::collector_factory::collector::MetricLike;
use betterlite_prometheus::no_op_metric::NoOpMetric;
use chrono::{DateTime, Utc};
use tokio::runtime::{Builder, Runtime};

use super::lookups::ScriptedLookups;
use super::output::samples_output;
use super::probe::{ProbeLog, RecordingGauge};

pub(super) const TEAM_REMAINING: &str = "litellm_remaining_team_budget_metric";
pub(super) const TEAM_MAX: &str = "litellm_team_max_budget_metric";
pub(super) const TEAM_HOURS: &str = "litellm_team_budget_remaining_hours_metric";
pub(super) const ORG_REMAINING: &str = "litellm_remaining_org_budget_metric";
pub(super) const ORG_MAX: &str = "litellm_org_max_budget_metric";
pub(super) const ORG_HOURS: &str = "litellm_org_budget_remaining_hours_metric";
pub(super) const KEY_REMAINING: &str = "litellm_remaining_api_key_budget_metric";
pub(super) const KEY_MAX: &str = "litellm_api_key_max_budget_metric";
pub(super) const KEY_HOURS: &str = "litellm_api_key_budget_remaining_hours_metric";
pub(super) const USER_REMAINING: &str = "litellm_remaining_user_budget_metric";
pub(super) const USER_MAX: &str = "litellm_user_max_budget_metric";
pub(super) const USER_HOURS: &str = "litellm_user_budget_remaining_hours_metric";
pub(super) const CUSTOMER_REMAINING: &str = "litellm_remaining_customer_budget_metric";
pub(super) const CUSTOMER_MAX: &str = "litellm_customer_max_budget_metric";
pub(super) const CUSTOMER_HOURS: &str = "litellm_customer_budget_remaining_hours_metric";
pub(super) const TEAM_GAUGES: [&str; 3] = [TEAM_REMAINING, TEAM_MAX, TEAM_HOURS];
pub(super) const ORG_GAUGES: [&str; 3] = [ORG_REMAINING, ORG_MAX, ORG_HOURS];
pub(super) const KEY_GAUGES: [&str; 3] = [KEY_REMAINING, KEY_MAX, KEY_HOURS];
pub(super) const USER_GAUGES: [&str; 3] = [USER_REMAINING, USER_MAX, USER_HOURS];
pub(super) const CUSTOMER_GAUGES: [&str; 3] = [CUSTOMER_REMAINING, CUSTOMER_MAX, CUSTOMER_HOURS];
pub(super) const SHAPE_ERROR_MESSAGE: &str = "the call shape does not match its function";

pub(super) fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

pub(super) fn shape_error() -> Box<HelperError> {
    helper_error("InvalidInput", SHAPE_ERROR_MESSAGE)
}

pub(super) fn runtime_error(message: &str) -> Box<HelperError> {
    helper_error("RuntimeError", message)
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| runtime_error(&error.to_string()))
}

// runs one setter on a fresh runtime and answers with the samples it recorded and the lookups it made
pub(super) fn finish<E, Emission>(
    probe: &ProbeLog,
    lookups: &ScriptedLookups,
    emission: Emission,
) -> HelperResult
where
    E: Display,
    Emission: Future<Output = Result<(), E>>,
{
    runtime()?
        .block_on(emission)
        .map_err(|error| runtime_error(&error.to_string()))?;
    Ok(samples_output(&probe.drain(), &lookups.calls()))
}

pub(super) fn entity_gauges(
    metrics: [&'static str; 3],
    labels: &BTreeMap<String, Vec<String>>,
    noop: bool,
    log: &ProbeLog,
) -> Result<EntityBudgetGauges<MetricLike<RecordingGauge>>, Box<HelperError>> {
    let [remaining, max, hours] = metrics;
    Ok(EntityBudgetGauges {
        remaining: field(remaining, labels, noop, log)?,
        max: field(max, labels, noop, log)?,
        remaining_hours: field(hours, labels, noop, log)?,
    })
}

fn field(
    metric: &'static str,
    labels: &BTreeMap<String, Vec<String>>,
    noop: bool,
    log: &ProbeLog,
) -> Result<GaugeField<MetricLike<RecordingGauge>>, Box<HelperError>> {
    if noop {
        return Ok(GaugeField {
            metric_name: metric,
            supported_labels: Vec::new(),
            collector: MetricLike::NoOp(NoOpMetric),
        });
    }
    let supported_labels = labels.get(metric).cloned().ok_or_else(|| {
        helper_error("InvalidInput", "the call has no label list for this metric")
    })?;
    Ok(GaugeField {
        metric_name: metric,
        supported_labels,
        collector: MetricLike::Real(RecordingGauge {
            metric,
            labels: Vec::new(),
            log: log.clone(),
        }),
    })
}

pub(super) const fn label_source(
    settings: &PrometheusLabelSettings,
    disable_end_user_cost_tracking: Option<bool>,
    enable_prometheus_only: Option<bool>,
) -> LabelSource<'_> {
    LabelSource::Values {
        settings,
        end_user_settings: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking,
            enable_prometheus_only,
        },
    }
}

pub(super) fn customer_context<'borrow>(
    settings: &'borrow PrometheusLabelSettings,
    end_user: EndUserCostTrackingSettings,
    now: DateTime<Utc>,
    tracker: &'borrow BoundedPrometheusSeriesTracker,
    limits: &'borrow SeriesLimits,
) -> CustomerGaugeContext<'borrow> {
    CustomerGaugeContext {
        source: label_source(
            settings,
            end_user.disable_end_user_cost_tracking,
            end_user.enable_prometheus_only,
        ),
        clock: BudgetGaugeClock {
            reset: now,
            monotonic: Instant::now(),
        },
        tracker,
        limits,
    }
}

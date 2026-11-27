#![cfg(test)]

use std::io::{Result as IoResult, Write};
use std::str::FromStr as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_prometheus::collector_factory::collector::{MetricLike, MetricValue};
use betterlite_prometheus::collector_registry::metric::{CollectorKind, PrometheusMetric};
use betterlite_prometheus::no_op_metric::NoOpMetric;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus::budget_gauges::customer::{BudgetGaugeClock, CustomerGaugeContext};
use crate::prometheus::budget_gauges::gauge::{EntityBudgetGauges, GaugeField};
use crate::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use crate::prometheus::budget_gauges::reset_hours::hours_until_reset;
use crate::prometheus::budget_lookups::lookups::{
    BudgetLookupError, BudgetLookups, UserLookupOptions,
};
use crate::prometheus::budget_lookups::records::{
    CachedEndUser, KeyRecord, OrgRecord, TeamRecord, UserRecord,
};
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, SeriesLimits,
};
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;
use crate::prometheus_label_schema::PrometheusLabelSettings;

type LookupResult<T> = Result<T, BudgetLookupError>;

pub(crate) const TEAM_GAUGES: [&str; 3] = [
    "litellm_remaining_team_budget_metric",
    "litellm_team_max_budget_metric",
    "litellm_team_budget_remaining_hours_metric",
];
pub(crate) const ORG_GAUGES: [&str; 3] = [
    "litellm_remaining_org_budget_metric",
    "litellm_org_max_budget_metric",
    "litellm_org_budget_remaining_hours_metric",
];
pub(crate) const KEY_GAUGES: [&str; 3] = [
    "litellm_remaining_api_key_budget_metric",
    "litellm_api_key_max_budget_metric",
    "litellm_api_key_budget_remaining_hours_metric",
];
pub(crate) const USER_GAUGES: [&str; 3] = [
    "litellm_remaining_user_budget_metric",
    "litellm_user_max_budget_metric",
    "litellm_user_budget_remaining_hours_metric",
];
pub(crate) const CUSTOMER_GAUGES: [&str; 3] = [
    "litellm_remaining_customer_budget_metric",
    "litellm_customer_max_budget_metric",
    "litellm_customer_budget_remaining_hours_metric",
];

pub(crate) fn decimal(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

pub(crate) fn time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

pub(crate) const fn default_tracking() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: None,
        enable_prometheus_only: None,
    }
}

pub(crate) const fn customer_tracking() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: None,
        enable_prometheus_only: Some(true),
    }
}

pub(crate) const fn source<'labels>(
    settings: &'labels PrometheusLabelSettings,
    end_user_settings: EndUserCostTrackingSettings,
) -> LabelSource<'labels> {
    LabelSource::Values {
        settings,
        end_user_settings,
    }
}

pub(crate) fn entity_gauges(
    names: [&'static str; 3],
    labels: &[&str],
    noop: bool,
) -> EntityBudgetGauges<MetricLike<PrometheusMetric>> {
    EntityBudgetGauges {
        remaining: field(names[0], labels, noop),
        max: field(names[1], labels, noop),
        remaining_hours: field(names[2], labels, noop),
    }
}

fn field(
    metric: &'static str,
    labels: &[&str],
    noop: bool,
) -> GaugeField<MetricLike<PrometheusMetric>> {
    let supported_labels: Vec<String> = labels.iter().map(|label| (*label).to_owned()).collect();
    let collector = if noop {
        MetricLike::NoOp(NoOpMetric)
    } else {
        let names: Vec<&str> = supported_labels.iter().map(String::as_str).collect();
        MetricLike::Real(PrometheusMetric::new(
            CollectorKind::Gauge,
            metric,
            metric,
            &names,
            Vec::new(),
        ))
    };
    GaugeField {
        metric_name: metric,
        supported_labels,
        collector,
    }
}

// the clock, tracker and settings a customer gauge write needs; the tracker keeps no limits, so no series is ever evicted
pub(crate) fn customer_context<'context>(
    settings: &'context PrometheusLabelSettings,
    tracker: &'context BoundedPrometheusSeriesTracker,
    limits: &'context SeriesLimits,
    tracking: EndUserCostTrackingSettings,
    now: DateTime<Utc>,
) -> CustomerGaugeContext<'context> {
    CustomerGaugeContext {
        source: source(settings, tracking),
        clock: BudgetGaugeClock {
            reset: now,
            monotonic: Instant::now(),
        },
        tracker,
        limits,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SampleRow {
    pub(crate) metric: &'static str,
    pub(crate) labels: Vec<(String, String)>,
    pub(crate) value: MetricValue,
}

// a produced series, read back the way a scrape sees it: the field's label names zipped with the stored label values
pub(crate) fn samples(field: &GaugeField<MetricLike<PrometheusMetric>>) -> Vec<SampleRow> {
    let MetricLike::Real(metric) = &field.collector else {
        return Vec::new();
    };
    let names = &metric.description.label_names;
    let mut rows = Vec::new();
    for (values, sample) in metric.series_in_use_order() {
        rows.push(SampleRow {
            metric: field.metric_name,
            labels: names.iter().cloned().zip(values).collect(),
            value: sample.total,
        });
    }
    rows
}

pub(crate) fn all_samples(
    gauges: &EntityBudgetGauges<MetricLike<PrometheusMetric>>,
) -> Vec<SampleRow> {
    let mut rows = samples(&gauges.remaining);
    rows.extend(samples(&gauges.max));
    rows.extend(samples(&gauges.remaining_hours));
    rows
}

fn labelled(metric: &'static str, labels: &[(&str, &str)], value: MetricValue) -> SampleRow {
    SampleRow {
        metric,
        labels: labels
            .iter()
            .map(|(name, label)| ((*name).to_owned(), (*label).to_owned()))
            .collect(),
        value,
    }
}

pub(crate) fn row(metric: &'static str, labels: &[(&str, &str)], value: &str) -> SampleRow {
    labelled(metric, labels, MetricValue::Finite(decimal(value)))
}

pub(crate) fn infinity_row(metric: &'static str, labels: &[(&str, &str)]) -> SampleRow {
    labelled(metric, labels, MetricValue::PositiveInfinity)
}

pub(crate) fn hours_row(
    metric: &'static str,
    labels: &[(&str, &str)],
    reset_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> SampleRow {
    let hours = hours_until_reset(reset_at, now);
    labelled(
        metric,
        labels,
        MetricValue::Finite(Decimal::from_f64(hours).unwrap()),
    )
}

#[derive(Clone, Debug)]
pub(crate) enum Answer<T> {
    Found(T),
    Missing,
    Failed(&'static str),
}

impl<T> Default for Answer<T> {
    fn default() -> Self {
        Self::Missing
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LookupCall {
    TeamInfo {
        team_id: String,
    },
    OrgInfo {
        org_id: String,
        include_budget_table: bool,
    },
    KeyInfo {
        hashed_token: String,
        check_cache_only: bool,
    },
    UserInfo {
        user_id: String,
        options: UserLookupOptions,
    },
    CachedCustomer {
        cache_key: String,
    },
    DefaultCustomerBudget {
        budget_id: String,
    },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RecordingLookups {
    pub(crate) database: bool,
    pub(crate) team: Answer<TeamRecord>,
    pub(crate) org: Answer<OrgRecord>,
    pub(crate) key: Answer<KeyRecord>,
    pub(crate) user: Answer<UserRecord>,
    pub(crate) customer: Answer<CachedEndUser>,
    pub(crate) default_budget: Answer<BudgetTableSnapshot>,
    calls: Arc<Mutex<Vec<LookupCall>>>,
}

impl RecordingLookups {
    pub(crate) fn with_database(mut self, database: bool) -> Self {
        self.database = database;
        self
    }

    pub(crate) fn with_team(mut self, team: Answer<TeamRecord>) -> Self {
        self.team = team;
        self
    }

    pub(crate) fn with_org(mut self, org: Answer<OrgRecord>) -> Self {
        self.org = org;
        self
    }

    pub(crate) fn with_key(mut self, key: Answer<KeyRecord>) -> Self {
        self.key = key;
        self
    }

    pub(crate) fn with_user(mut self, user: Answer<UserRecord>) -> Self {
        self.user = user;
        self
    }

    pub(crate) fn with_customer(mut self, customer: Answer<CachedEndUser>) -> Self {
        self.customer = customer;
        self
    }

    pub(crate) fn with_default_budget(
        mut self,
        default_budget: Answer<BudgetTableSnapshot>,
    ) -> Self {
        self.default_budget = default_budget;
        self
    }

    pub(crate) fn calls(&self) -> Vec<LookupCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: LookupCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn failure(message: &'static str) -> BudgetLookupError {
    BudgetLookupError {
        message: message.to_owned(),
    }
}

fn answer<T>(answer: Answer<T>) -> Result<Option<T>, BudgetLookupError> {
    match answer {
        Answer::Found(value) => Ok(Some(value)),
        Answer::Missing => Ok(None),
        Answer::Failed(message) => Err(failure(message)),
    }
}

// a lookup that raises on a miss, the way get_team_object and the cache-only key read do
fn required<T>(answer: Answer<T>) -> Result<T, BudgetLookupError> {
    match answer {
        Answer::Found(value) => Ok(value),
        Answer::Missing => Err(failure("the test script has no record for this lookup")),
        Answer::Failed(message) => Err(failure(message)),
    }
}

#[async_trait]
impl BudgetLookups for RecordingLookups {
    fn database_configured(&self) -> bool {
        self.database
    }

    async fn team_info(&self, team_id: &str) -> LookupResult<TeamRecord> {
        self.record(LookupCall::TeamInfo {
            team_id: team_id.to_owned(),
        });
        required(self.team.clone())
    }

    async fn org_info(&self, org_id: &str, include: bool) -> LookupResult<Option<OrgRecord>> {
        self.record(LookupCall::OrgInfo {
            org_id: org_id.to_owned(),
            include_budget_table: include,
        });
        answer(self.org.clone())
    }

    async fn key_info(
        &self,
        hashed_token: &str,
        check_cache_only: bool,
    ) -> LookupResult<KeyRecord> {
        self.record(LookupCall::KeyInfo {
            hashed_token: hashed_token.to_owned(),
            check_cache_only,
        });
        required(self.key.clone())
    }

    async fn user_info(
        &self,
        user_id: &str,
        options: UserLookupOptions,
    ) -> LookupResult<Option<UserRecord>> {
        self.record(LookupCall::UserInfo {
            user_id: user_id.to_owned(),
            options,
        });
        answer(self.user.clone())
    }

    async fn cached_customer(&self, end_user_id: &str) -> LookupResult<Option<CachedEndUser>> {
        self.record(LookupCall::CachedCustomer {
            cache_key: format!("end_user_id:{end_user_id}"),
        });
        answer(self.customer.clone())
    }

    async fn default_customer_budget(
        &self,
        budget_id: &str,
    ) -> LookupResult<Option<BudgetTableSnapshot>> {
        self.record(LookupCall::DefaultCustomerBudget {
            budget_id: budget_id.to_owned(),
        });
        answer(self.default_budget.clone())
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("captured logs are utf-8")
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

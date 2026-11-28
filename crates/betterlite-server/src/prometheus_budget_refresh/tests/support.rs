use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::DeletedVerificationToken;
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::customer::{
    BudgetGaugeClock, CustomerGaugeContext,
};
use betterlite_logging::prometheus::budget_gauges::gauge::{
    BudgetGauges, EntityBudgetGauges, GaugeField,
};
use betterlite_logging::prometheus::budget_gauges::inputs::{
    TeamBudgetGaugeInput, UserBudgetGaugeInput,
};
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_gauges::lists::customer_row::CustomerBudgetGaugeRow;
use betterlite_logging::prometheus::budget_gauges::lists::org_list::OrgBudgetGaugeRow;
use betterlite_logging::prometheus::budget_lookups::lookups::{
    BudgetLookupError, BudgetLookups, UserLookupOptions,
};
use betterlite_logging::prometheus::budget_lookups::records::{
    CachedEndUser, KeyRecord, OrgRecord, TeamRecord, UserRecord,
};
use betterlite_logging::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric, SeriesLimits,
};
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::{
    Collector, MetricLike, MetricUpdate, MetricValue,
};
use betterlite_prometheus::no_op_metric::NoOpMetric;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;

use crate::key_query_plan::response::KeyListEntry as KeyPageEntry;
use crate::prometheus_budget_refresh::refresh::BudgetRefreshLock;
use crate::prometheus_budget_refresh::source::{BudgetPageError, BudgetPageSource, Page};

// One Set write, with the metric it landed on and the labels it carried.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetEvent {
    pub metric: String,
    pub labels: Vec<(String, String)>,
    pub value: MetricValue,
}

// Every gauge event a test wrote, shared with the gauges built over it.
#[derive(Clone, Default)]
pub struct ProbeLog {
    events: Arc<Mutex<Vec<SetEvent>>>,
}

impl ProbeLog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn events(&self) -> Vec<SetEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[must_use]
    pub fn for_metric(&self, metric: &str) -> Vec<SetEvent> {
        self.events()
            .into_iter()
            .filter(|event| event.metric == metric)
            .collect()
    }

    fn record(&self, event: SetEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

// A gauge that records every Set instead of scraping a registry.
#[derive(Clone, Debug)]
pub struct ProbeGauge {
    metric: &'static str,
    labels: Vec<(String, String)>,
    log: ProbeLog,
}

impl Collector for ProbeGauge {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let mut next = self.clone();
        for (name, value) in labels {
            next.labels.push(((*name).to_owned(), (*value).to_owned()));
        }
        next
    }

    fn update(&self, update: MetricUpdate) {
        if let MetricUpdate::Set(value) = update {
            self.log.record(SetEvent {
                metric: self.metric.to_owned(),
                labels: self.labels.clone(),
                value,
            });
        }
    }
}

impl RemovableMetric for ProbeGauge {
    type Error = Infallible;

    fn remove_child(&self, _label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        Ok(ChildRemoval::Removed)
    }
}

#[must_use]
pub fn real_gauge(metric: &'static str, log: &ProbeLog) -> MetricLike<ProbeGauge> {
    MetricLike::Real(ProbeGauge {
        metric,
        labels: Vec::new(),
        log: log.clone(),
    })
}

fn field(
    metric: &'static str,
    supported: &[&str],
    log: &ProbeLog,
) -> GaugeField<MetricLike<ProbeGauge>> {
    GaugeField {
        metric_name: metric,
        supported_labels: supported.iter().map(|label| (*label).to_owned()).collect(),
        collector: real_gauge(metric, log),
    }
}

const fn noop_field(metric: &'static str) -> GaugeField<MetricLike<ProbeGauge>> {
    GaugeField {
        metric_name: metric,
        supported_labels: Vec::new(),
        collector: MetricLike::NoOp(NoOpMetric),
    }
}

#[must_use]
pub fn entity(labels: &[&str], log: &ProbeLog) -> EntityBudgetGauges<MetricLike<ProbeGauge>> {
    EntityBudgetGauges {
        remaining: field("remaining", labels, log),
        max: field("max", labels, log),
        remaining_hours: field("hours", labels, log),
    }
}

#[must_use]
pub const fn noop_entity() -> EntityBudgetGauges<MetricLike<ProbeGauge>> {
    EntityBudgetGauges {
        remaining: noop_field("remaining"),
        max: noop_field("max"),
        remaining_hours: noop_field("hours"),
    }
}

// The five entity gauge groups plus one log every group writes to.
pub struct BudgetRig {
    pub gauges: BudgetGauges<MetricLike<ProbeGauge>>,
    pub log: ProbeLog,
}

#[must_use]
pub fn budget_rig(labels: &[&str]) -> BudgetRig {
    let log = ProbeLog::new();
    BudgetRig {
        gauges: BudgetGauges {
            team: entity(labels, &log),
            org: entity(labels, &log),
            key: entity(labels, &log),
            user: entity(labels, &log),
            customer: entity(labels, &log),
            team_members: field("team_members", labels, &log),
            provider_remaining: field("provider_remaining", labels, &log),
        },
        log,
    }
}

#[must_use]
pub fn team(
    team_id: &str,
    alias: Option<&str>,
    spend: Option<Decimal>,
    max_budget: Option<Decimal>,
) -> TeamBudgetGaugeInput {
    TeamBudgetGaugeInput {
        team_id: team_id.to_owned(),
        team_alias: alias.map(str::to_owned),
        spend,
        max_budget,
        budget_reset_at: None,
    }
}

#[must_use]
pub fn user(user_id: &str, spend: Decimal, max_budget: Option<Decimal>) -> UserBudgetGaugeInput {
    UserBudgetGaugeInput {
        user_id: user_id.to_owned(),
        user_email: None,
        user_alias: None,
        spend,
        max_budget,
        budget_reset_at: None,
    }
}

#[must_use]
pub fn org(
    org_id: &str,
    spend: Option<Decimal>,
    budget: Option<BudgetTableSnapshot>,
) -> OrgBudgetGaugeRow {
    let alias = format!("{org_id}-alias");
    OrgBudgetGaugeRow {
        organization_id: Some(org_id.to_owned()),
        organization_alias: Some(alias),
        spend,
        budget,
    }
}

#[must_use]
pub fn customer(
    user_id: &str,
    spend: Decimal,
    budget: Option<BudgetTableSnapshot>,
) -> CustomerBudgetGaugeRow {
    CustomerBudgetGaugeRow {
        user_id: user_id.to_owned(),
        spend,
        budget,
    }
}

#[must_use]
pub fn snapshot(
    max_budget: Option<Decimal>,
    reset_at: Option<DateTime<Utc>>,
) -> BudgetTableSnapshot {
    BudgetTableSnapshot {
        max_budget,
        budget_reset_at: reset_at,
    }
}

#[must_use]
pub fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_893_456_000, 0).unwrap()
}

#[must_use]
pub fn clock() -> BudgetGaugeClock {
    BudgetGaugeClock {
        reset: now(),
        monotonic: Instant::now(),
    }
}

#[must_use]
pub const fn no_limits() -> SeriesLimits {
    SeriesLimits {
        max_series: None,
        ttl: None,
        cleanup_interval: None,
    }
}

#[must_use]
pub const fn prometheus_only() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: None,
        enable_prometheus_only: Some(true),
    }
}

#[must_use]
pub const fn end_user_tracking_disabled() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: Some(true),
        enable_prometheus_only: Some(true),
    }
}

#[must_use]
pub const fn prometheus_only_off() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: None,
        enable_prometheus_only: Some(false),
    }
}

#[must_use]
pub const fn labels_source(
    settings: &PrometheusLabelSettings,
    end_user: EndUserCostTrackingSettings,
) -> LabelSource<'_> {
    LabelSource::Values {
        settings,
        end_user_settings: end_user,
    }
}

#[must_use]
pub const fn customer_context<'context>(
    labels: LabelSource<'context>,
    clock: BudgetGaugeClock,
    tracker: &'context BoundedPrometheusSeriesTracker,
    limits: &'context SeriesLimits,
) -> CustomerGaugeContext<'context> {
    CustomerGaugeContext {
        source: labels,
        clock,
        tracker,
        limits,
    }
}

// Captured tracing output, written by a subscriber installed for the current thread.
#[derive(Clone, Default)]
pub struct LogBuffer {
    written: Arc<Mutex<Vec<u8>>>,
}

impl LogBuffer {
    #[must_use]
    pub fn text(&self) -> String {
        let bytes = self
            .written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap_or_default()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

#[must_use]
pub fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (buffer, guard)
}

fn key_fields(
    token: &str,
    alias: &str,
    spend: Decimal,
    max_budget: Option<Decimal>,
) -> BTreeMap<String, JsonData> {
    let mut fields = BTreeMap::new();
    fields.insert("token".to_owned(), JsonData::String(token.to_owned()));
    fields.insert("key_alias".to_owned(), JsonData::String(alias.to_owned()));
    fields.insert("spend".to_owned(), JsonData::String(spend.to_string()));
    if let Some(limit) = max_budget {
        fields.insert("max_budget".to_owned(), JsonData::String(limit.to_string()));
    }
    fields
}

#[must_use]
pub fn auth_entry(
    token: &str,
    alias: &str,
    spend: Decimal,
    max_budget: Option<Decimal>,
) -> KeyPageEntry {
    let auth =
        UserAPIKeyAuth::construct(AuthInput::Dict(key_fields(token, alias, spend, max_budget)))
            .unwrap();
    KeyPageEntry::Auth(Box::new(auth))
}

#[must_use]
pub fn token_entry(token: &str) -> KeyPageEntry {
    KeyPageEntry::Token(token.to_owned())
}

#[must_use]
pub fn deleted_entry(token: &str) -> KeyPageEntry {
    let mut fields = BTreeMap::new();
    fields.insert("token".to_owned(), JsonData::String(token.to_owned()));
    let text = serde_json::to_string(&JsonData::Object(fields)).unwrap();
    let stored: DeletedVerificationToken = serde_json::from_str(&text).unwrap();
    KeyPageEntry::Deleted(Box::new(stored))
}

type PageResult<Row> = Result<Page<Row>, BudgetPageError>;
type Scripted<Row> = Mutex<VecDeque<PageResult<Row>>>;

// A page source scripted per entity; an unscripted read is an empty page.
#[derive(Default)]
pub struct FakeSource {
    teams: Scripted<TeamBudgetGaugeInput>,
    keys: Scripted<KeyPageEntry>,
    users: Scripted<UserBudgetGaugeInput>,
    orgs: Scripted<OrgBudgetGaugeRow>,
    customers: Scripted<CustomerBudgetGaugeRow>,
    counts: Mutex<VecDeque<Result<u64, BudgetPageError>>>,
    calls: Mutex<Vec<String>>,
}

impl FakeSource {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn push_teams(&self, page: PageResult<TeamBudgetGaugeInput>) {
        self.teams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(page);
    }

    pub fn push_keys(&self, page: PageResult<KeyPageEntry>) {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(page);
    }

    pub fn push_users(&self, page: PageResult<UserBudgetGaugeInput>) {
        self.users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(page);
    }

    pub fn push_orgs(&self, page: PageResult<OrgBudgetGaugeRow>) {
        self.orgs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(page);
    }

    pub fn push_customers(&self, page: PageResult<CustomerBudgetGaugeRow>) {
        self.customers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(page);
    }

    pub fn push_count(&self, count: Result<u64, BudgetPageError>) {
        self.counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(count);
    }

    fn record(&self, call: String) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn page<Row>(&self, label: String, queue: &Scripted<Row>) -> PageResult<Row> {
        self.record(label);
        Self::next_page(queue)
    }

    fn next_page<Row>(queue: &Scripted<Row>) -> PageResult<Row> {
        match queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            Some(page) => page,
            None => Ok(Page {
                rows: Vec::new(),
                total_count: Some(0),
            }),
        }
    }

    fn next_count(&self) -> Result<u64, BudgetPageError> {
        match self
            .counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            Some(count) => count,
            None => Ok(0),
        }
    }
}

#[async_trait]
impl BudgetPageSource for FakeSource {
    async fn teams_page(&self, page_size: u64, page: u64) -> PageResult<TeamBudgetGaugeInput> {
        self.page(format!("teams:{page_size}:{page}"), &self.teams)
    }

    async fn keys_page(&self, page_size: u64, page: u64) -> PageResult<KeyPageEntry> {
        self.page(format!("keys:{page_size}:{page}"), &self.keys)
    }

    async fn users_page(&self, page_size: u64, page: u64) -> PageResult<UserBudgetGaugeInput> {
        self.page(format!("users:{page_size}:{page}"), &self.users)
    }

    async fn orgs_page(&self, page_size: u64, page: u64) -> PageResult<OrgBudgetGaugeRow> {
        self.page(format!("orgs:{page_size}:{page}"), &self.orgs)
    }

    async fn customers_page(
        &self,
        page_size: u64,
        page: u64,
        budgeted_only: bool,
    ) -> PageResult<CustomerBudgetGaugeRow> {
        let label = format!("customers:{page_size}:{page}:{budgeted_only}");
        self.page(label, &self.customers)
    }

    async fn user_count(&self) -> Result<u64, BudgetPageError> {
        self.record("user_count".to_owned());
        self.next_count()
    }

    async fn billable_user_count(&self) -> Result<u64, BudgetPageError> {
        self.record("billable_user_count".to_owned());
        self.next_count()
    }

    async fn team_count(&self) -> Result<u64, BudgetPageError> {
        self.record("team_count".to_owned());
        self.next_count()
    }
}

// The lookups the customer refresh makes: only the default budget read is answered.
pub struct FakeLookups {
    default_budget: Mutex<Option<Result<Option<BudgetTableSnapshot>, BudgetLookupError>>>,
    default_calls: Mutex<Vec<String>>,
}

impl FakeLookups {
    #[must_use]
    pub fn with_budget(budget: Option<BudgetTableSnapshot>) -> Self {
        Self {
            default_budget: Mutex::new(Some(Ok(budget))),
            default_calls: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn failing() -> Self {
        Self {
            default_budget: Mutex::new(Some(Err(BudgetLookupError {
                message: "lookup down".to_owned(),
            }))),
            default_calls: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn default_calls(&self) -> Vec<String> {
        self.default_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn unused_lookup() -> BudgetLookupError {
    BudgetLookupError {
        message: "the refresh does not call this lookup".to_owned(),
    }
}

#[async_trait]
impl BudgetLookups for FakeLookups {
    fn database_configured(&self) -> bool {
        true
    }

    async fn team_info(&self, _team_id: &str) -> Result<TeamRecord, BudgetLookupError> {
        Err(unused_lookup())
    }

    async fn org_info(
        &self,
        _org_id: &str,
        _include_budget_table: bool,
    ) -> Result<Option<OrgRecord>, BudgetLookupError> {
        Err(unused_lookup())
    }

    async fn key_info(
        &self,
        _hashed_token: &str,
        _check_cache_only: bool,
    ) -> Result<KeyRecord, BudgetLookupError> {
        Err(unused_lookup())
    }

    async fn user_info(
        &self,
        _user_id: &str,
        _options: UserLookupOptions,
    ) -> Result<Option<UserRecord>, BudgetLookupError> {
        Err(unused_lookup())
    }

    async fn cached_customer(
        &self,
        _end_user_id: &str,
    ) -> Result<Option<CachedEndUser>, BudgetLookupError> {
        Err(unused_lookup())
    }

    async fn default_customer_budget(
        &self,
        budget_id: &str,
    ) -> Result<Option<BudgetTableSnapshot>, BudgetLookupError> {
        self.default_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(budget_id.to_owned());
        match self
            .default_budget
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            Some(answer) => answer,
            None => Ok(None),
        }
    }
}

// A pod lock whose cache and answer the test picks.
pub struct FakeLock {
    cache: bool,
    answer: Option<bool>,
    calls: Mutex<Vec<String>>,
}

impl FakeLock {
    #[must_use]
    pub fn with_cache(answer: Option<bool>) -> Self {
        Self {
            cache: true,
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn without_cache() -> Self {
        Self {
            cache: false,
            answer: Some(true),
            calls: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl BudgetRefreshLock for FakeLock {
    fn has_cache(&self) -> bool {
        self.cache
    }

    async fn acquire_lock(&self, cronjob_id: &'static str) -> Option<bool> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("acquire:{cronjob_id}"));
        self.answer
    }

    async fn release_lock(&self, cronjob_id: &'static str) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("release:{cronjob_id}"));
    }
}

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_end_users::defaults::MemberBudgetError;
use betterlite_auth::auth_key_budgets::alerts::{
    BudgetAlert, BudgetAlertKind, BudgetAlertSink, EventGroup,
};
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::team_budget_admission::alerts::{TeamAlert, TeamAlertKind, TeamAlertSink};
use betterlite_auth::team_budget_admission::sources::{DefaultBudgetSource, MembershipSource};
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable, Timestamp};
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_error::proxy_exception::ProxyException;
use chrono::{DateTime, SecondsFormat, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::{Number, Value, json};

// the case's counter values, handed out in read order, and the team default the case seeds into LiteLLM's cache
pub(crate) struct Seeded {
    spends: Vec<JsonData>,
    default_budget: Option<LiteLLMBudgetTable>,
    pub(crate) reads: Mutex<Vec<Value>>,
}

impl Seeded {
    pub(crate) fn new(spends: &[JsonData], default_budget: Option<LiteLLMBudgetTable>) -> Self {
        Self {
            spends: spends.to_vec(),
            default_budget,
            reads: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SpendReader for Seeded {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        let scripted = self.spends.get(reads.len());
        let window = query.window;
        reads.push(json!({
            "counter_key": query.counter_key,
            "fallback_spend": float(query.fallback_spend),
            "max_budget": query.max_budget.map_or(Value::Null, float),
            "entity_type": window.entity_type.map(|kind| format!("{kind:?}")),
            "entity_id": window.entity_id,
            "duration": window.duration,
            "start": window.start.map(isoformat),
        }));
        // a read past the script is the RuntimeError the Python side raises
        scripted
            .and_then(Money::from_json)
            .and_then(Money::finite_value)
            .ok_or_else(|| SpendReadError::Unverifiable {
                counter_key: query.counter_key.to_owned(),
            })
    }
}

// LiteLLM is handed the membership as already loaded, so a query here is a defect the answer shows
#[async_trait]
impl MembershipSource for Seeded {
    async fn team_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Err(MembershipLoadError::NoDatabase)
    }
}

#[async_trait]
impl DefaultBudgetSource for Seeded {
    async fn team_member_default(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        Ok(self
            .default_budget
            .clone()
            .filter(|budget| budget.budget_id.as_deref() == Some(budget_id)))
    }
}

// LiteLLM's duration_in_seconds, which the window start uses
pub(crate) struct CaseDurations(pub(crate) TimeZoneDatabase);

impl DurationParser for CaseDurations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        let parsed = duration_in_seconds(spec, now, None, &self.0).ok()?;
        Some(parsed.seconds)
    }
}

// both sinks write one list, in the order LiteLLM's create_task calls would run
#[derive(Default)]
pub(crate) struct Recorder(pub(crate) Mutex<Vec<Value>>);

// either sink's alert, so one place builds the CallInfo both traits write
enum RecordedAlert {
    Team(TeamAlert),
    Budget(BudgetAlert),
}

impl Recorder {
    fn record(&self, kind: &'static str, alert: RecordedAlert) {
        let info = match alert {
            RecordedAlert::Team(team) => CallInfo {
                spend: float(team.spend),
                max_budget: team.max_budget.map_or(Value::Null, float),
                soft_budget: team.soft_budget.map_or(Value::Null, float),
                token: team.token.map(|secret| secret.expose_secret().to_owned()),
                user_id: team.user_id,
                team_id: team.team_id,
                team_alias: team.team_alias,
                organization_id: team.organization_id,
                user_email: None,
                key_alias: team.key_alias,
                event_group: "team",
                alert_emails: team.alert_emails,
                max_budget_alert_emails: None,
                unset: UnsetCallInfo::default(),
            },
            RecordedAlert::Budget(budget) => CallInfo {
                spend: float(budget.spend),
                max_budget: finite(budget.max_budget),
                soft_budget: finite(budget.soft_budget),
                token: budget.token.map(|secret| secret.expose_secret().to_owned()),
                user_id: budget.user_id,
                team_id: budget.team_id,
                team_alias: budget.team_alias,
                organization_id: budget.organization_id,
                user_email: budget.user_email,
                key_alias: budget.key_alias,
                event_group: event_group(budget.event_group),
                alert_emails: None,
                max_budget_alert_emails: budget.max_budget_alert_emails,
                unset: UnsetCallInfo::default(),
            },
        };
        self.push(kind, &info);
    }

    fn push(&self, kind: &str, info: &CallInfo) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"type": kind, "info": info}));
    }
}

// the budget_alerts type LiteLLM queues each kind under
const fn team_kind(kind: TeamAlertKind) -> &'static str {
    match kind {
        TeamAlertKind::TeamBudget => "team_budget",
        TeamAlertKind::SoftBudget => "soft_budget",
    }
}

const fn budget_kind(kind: BudgetAlertKind) -> &'static str {
    match kind {
        BudgetAlertKind::TokenBudget => "token_budget",
        BudgetAlertKind::SoftBudget => "soft_budget",
        BudgetAlertKind::MaxBudgetAlert => "max_budget_alert",
    }
}

const fn event_group(group: EventGroup) -> &'static str {
    match group {
        EventGroup::Key => "key",
        EventGroup::TeamMember => "team_member",
    }
}

impl TeamAlertSink for Recorder {
    fn schedule(&self, alert: TeamAlert) {
        self.record(team_kind(alert.kind), RecordedAlert::Team(alert));
    }
}

impl BudgetAlertSink for Recorder {
    fn schedule(&self, alert: BudgetAlert) {
        self.record(budget_kind(alert.kind), RecordedAlert::Budget(alert));
    }
}

// the CallInfo fields these team checks never set
#[derive(Default, Serialize)]
struct UnsetCallInfo {
    customer_id: Option<String>,
    projected_exceeded_date: Option<String>,
    projected_spend: Value,
}

// CallInfo.model_dump_json's fields
#[derive(Serialize)]
struct CallInfo {
    spend: Value,
    max_budget: Value,
    soft_budget: Value,
    token: Option<String>,
    user_id: Option<String>,
    team_id: Option<String>,
    team_alias: Option<String>,
    organization_id: Option<String>,
    user_email: Option<String>,
    key_alias: Option<String>,
    event_group: &'static str,
    alert_emails: Option<Vec<String>>,
    max_budget_alert_emails: Option<BTreeMap<String, Vec<String>>>,
    #[serde(flatten)]
    unset: UnsetCallInfo,
}

fn finite(money: Option<Money>) -> Value {
    money
        .and_then(Money::finite_value)
        .map_or(Value::Null, float)
}

// LiteLLM's money is a float, so a whole amount still reaches the oracle as 10.0
fn float(value: Decimal) -> Value {
    value
        .to_f64()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

// datetime.isoformat() for the whole-second starts the cases pin
fn isoformat(start: Timestamp) -> String {
    match start {
        Timestamp::Aware(moment) => moment.to_rfc3339_opts(SecondsFormat::AutoSi, false),
        Timestamp::Naive(moment) => moment.format("%Y-%m-%dT%H:%M:%S").to_string(),
    }
}

// BudgetExceededError's caller-visible fields
pub(crate) fn denial_json(denial: &Denial) -> Value {
    json!({
        "status": denial.status_code,
        "current_cost": float(denial.current_cost),
        "max_budget": float(denial.max_budget),
        "entity_type": denial.entity_type,
        "entity_id": denial.entity_id.as_ref().map(|entity| entity.expose_secret().to_owned()),
    })
}

// the ProxyException fields the auth error handler turns into the response body
pub(crate) fn exception_json(exception: &ProxyException) -> Value {
    json!({
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
    })
}

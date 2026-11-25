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

impl Recorder {
    fn push(&self, kind: &str, info: &CallInfo) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"type": kind, "info": info}));
    }
}

impl TeamAlertSink for Recorder {
    fn schedule(&self, alert: TeamAlert) {
        let kind = match alert.kind {
            TeamAlertKind::TeamBudget => "team_budget",
            TeamAlertKind::SoftBudget => "soft_budget",
        };
        let info = CallInfo {
            spend: float(alert.spend),
            max_budget: alert.max_budget.map_or(Value::Null, float),
            soft_budget: alert.soft_budget.map_or(Value::Null, float),
            token: alert.token.map(|secret| secret.expose_secret().to_owned()),
            customer_id: None,
            user_id: alert.user_id,
            team_id: alert.team_id,
            team_alias: alert.team_alias,
            organization_id: alert.organization_id,
            user_email: None,
            key_alias: alert.key_alias,
            projected_exceeded_date: None,
            projected_spend: Value::Null,
            event_group: "team",
            alert_emails: alert.alert_emails,
            max_budget_alert_emails: None,
        };
        self.push(kind, &info);
    }
}

impl BudgetAlertSink for Recorder {
    fn schedule(&self, alert: BudgetAlert) {
        let kind = match alert.kind {
            BudgetAlertKind::TokenBudget => "token_budget",
            BudgetAlertKind::SoftBudget => "soft_budget",
            BudgetAlertKind::MaxBudgetAlert => "max_budget_alert",
        };
        let event_group = match alert.event_group {
            EventGroup::Key => "key",
            EventGroup::TeamMember => "team_member",
        };
        let info = CallInfo {
            spend: float(alert.spend),
            max_budget: finite(alert.max_budget),
            soft_budget: finite(alert.soft_budget),
            token: alert.token.map(|secret| secret.expose_secret().to_owned()),
            customer_id: None,
            user_id: alert.user_id,
            team_id: alert.team_id,
            team_alias: alert.team_alias,
            organization_id: alert.organization_id,
            user_email: alert.user_email,
            key_alias: alert.key_alias,
            projected_exceeded_date: None,
            projected_spend: Value::Null,
            event_group,
            alert_emails: None,
            max_budget_alert_emails: alert.max_budget_alert_emails,
        };
        self.push(kind, &info);
    }
}

// CallInfo.model_dump_json's fields; the projections and the customer are never set by these checks
#[derive(Serialize)]
struct CallInfo {
    spend: Value,
    max_budget: Value,
    soft_budget: Value,
    token: Option<String>,
    customer_id: Option<String>,
    user_id: Option<String>,
    team_id: Option<String>,
    team_alias: Option<String>,
    organization_id: Option<String>,
    user_email: Option<String>,
    key_alias: Option<String>,
    projected_exceeded_date: Option<String>,
    projected_spend: Value,
    event_group: &'static str,
    alert_emails: Option<Vec<String>>,
    max_budget_alert_emails: Option<BTreeMap<String, Vec<String>>>,
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

use std::mem::take;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_key_budgets::alerts::{
    BudgetAlert, BudgetAlertKind, BudgetAlertSink, EventGroup,
};
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::budget_counter::CounterEntityType;
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_core::duration::seconds::duration_in_seconds;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::{DateTime, NaiveDateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::ExposeSecret as _;
use serde_json::{Number, Value, json};

// the spend reads the case scripts, recorded the way LiteLLM's get_current_spend receives them
pub(crate) struct ScriptedSpend {
    pub(crate) spends: Vec<JsonData>,
    pub(crate) next: AtomicUsize,
    pub(crate) reads: Mutex<Vec<Value>>,
}

impl ScriptedSpend {
    pub(crate) fn take(&self) -> Vec<Value> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        take(&mut *reads)
    }
}

#[async_trait]
impl SpendReader for ScriptedSpend {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        let index = self.next.fetch_add(1, Ordering::SeqCst);
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(read_json(query));
        let Some(scripted) = self.spends.get(index) else {
            return Err(unverifiable(query));
        };
        Money::from_json(scripted)
            .and_then(Money::finite_value)
            .ok_or_else(|| unverifiable(query))
    }
}

fn unverifiable(query: &SpendQuery<'_>) -> SpendReadError {
    SpendReadError::Unverifiable {
        counter_key: query.counter_key.to_owned(),
    }
}

// the alerts the checks schedule, kept until the call's answer reads them
#[derive(Default)]
pub(crate) struct Recorder(Mutex<Vec<BudgetAlert>>);

impl Recorder {
    pub(crate) fn take(&self) -> Vec<Value> {
        let alerts = take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
        alerts.iter().map(alert_json).collect()
    }
}

impl BudgetAlertSink for Recorder {
    fn schedule(&self, alert: BudgetAlert) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(alert);
    }
}

// LiteLLM's own duration parser, which resolves the window suffixes the budget limits carry
pub(crate) struct CaseDurations(pub(crate) TimeZoneDatabase);

impl DurationParser for CaseDurations {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        duration_in_seconds(spec, now, None, &self.0)
            .ok()
            .map(|duration| duration.seconds)
    }
}

fn read_json(query: &SpendQuery<'_>) -> Value {
    let (start, offset) = start_fields(query.window.start);
    json!({
        "counter_key": query.counter_key,
        "fallback_spend": decimal_json(query.fallback_spend),
        "max_budget": query.max_budget.map_or(Value::Null, decimal_json),
        "entity_type": query.window.entity_type.map(litellm_entity_name),
        "entity_id": query.window.entity_id,
        "duration": query.window.duration,
        "start": start,
        "offset_seconds": offset,
    })
}

// datetime.isoformat() on the naive local value, with the offset in seconds kept apart
fn start_fields(start: Option<Timestamp>) -> (Value, Option<i32>) {
    match start {
        None => (Value::Null, None),
        Some(Timestamp::Aware(moment)) => (
            Value::String(isoformat(&moment.naive_local())),
            Some(moment.offset().local_minus_utc()),
        ),
        Some(Timestamp::Naive(moment)) => (Value::String(isoformat(&moment)), None),
    }
}

fn isoformat(moment: &NaiveDateTime) -> String {
    let base = moment.format("%Y-%m-%dT%H:%M:%S").to_string();
    let micros = moment.and_utc().timestamp_subsec_micros();
    if micros == 0 {
        return base;
    }
    format!("{base}.{micros:06}")
}

fn alert_json(alert: &BudgetAlert) -> Value {
    let token = alert.token.as_ref().map_or(Value::Null, |token| {
        Value::String(token.expose_secret().to_owned())
    });
    json!({
        "type": alert_kind_name(alert.kind),
        "info": {
            "spend": decimal_json(alert.spend),
            "max_budget": money_json(alert.max_budget),
            "soft_budget": money_json(alert.soft_budget),
            "token": token,
            "customer_id": Value::Null,
            "user_id": alert.user_id,
            "team_id": alert.team_id,
            "team_alias": alert.team_alias,
            "organization_id": alert.organization_id,
            "user_email": alert.user_email,
            "key_alias": alert.key_alias,
            "projected_exceeded_date": Value::Null,
            "projected_spend": Value::Null,
            "event_group": alert_event_group(alert.event_group),
            "alert_emails": Value::Null,
            "max_budget_alert_emails": alert.max_budget_alert_emails,
        },
    })
}

const fn alert_kind_name(kind: BudgetAlertKind) -> &'static str {
    match kind {
        BudgetAlertKind::TokenBudget => "token_budget",
        BudgetAlertKind::SoftBudget => "soft_budget",
        BudgetAlertKind::MaxBudgetAlert => "max_budget_alert",
    }
}

const fn alert_event_group(group: EventGroup) -> &'static str {
    match group {
        EventGroup::Key => "key",
        EventGroup::TeamMember => "team_member",
    }
}

pub(crate) fn money_json(money: Option<Money>) -> Value {
    money
        .and_then(Money::finite_value)
        .map_or(Value::Null, decimal_json)
}

// LiteLLM's float fields reach the oracle as floats, even when the spend is a whole number
pub(crate) fn decimal_json(value: Decimal) -> Value {
    value
        .to_f64()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

// the reverse of CounterEntityType::from_litellm_name, which keeps only the forward direction public
const fn litellm_entity_name(entity_type: &CounterEntityType) -> &'static str {
    match entity_type {
        CounterEntityType::Key => "Key",
        CounterEntityType::Team => "Team",
        CounterEntityType::TeamMember => "TeamMember",
        CounterEntityType::User => "User",
        CounterEntityType::EndUser => "EndUser",
        CounterEntityType::Tag => "Tag",
        CounterEntityType::ModelAccessGroup => "Model access group",
        CounterEntityType::Organization => "Organization",
        CounterEntityType::Project => "Project",
    }
}

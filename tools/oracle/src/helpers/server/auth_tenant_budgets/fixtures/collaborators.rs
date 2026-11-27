use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_tenant_budgets::alerts::{
    TenantAlert, TenantAlertKind, TenantAlertSink, TenantEventGroup,
};
use betterlite_auth::current_spend::SpendQuery;
use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::{Number, Value, json};

// the counter values a case scripts, handed out in read order, and the query each read carried
pub(crate) struct Seeded {
    spends: Vec<JsonData>,
    reads: Mutex<Vec<Value>>,
}

impl Seeded {
    pub(crate) fn new(spends: &[JsonData]) -> Self {
        Self {
            spends: spends.to_vec(),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<Value> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl SpendReader for Seeded {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        let scripted = self.spends.get(reads.len());
        reads.push(json!({
            "counter_key": query.counter_key,
            "fallback_spend": float(query.fallback_spend),
            "max_budget": query.max_budget.map_or(Value::Null, float),
            "fallback_authoritative": query.fallback_authoritative,
        }));
        drop(reads);
        // a read past the script is the RuntimeError the Python side raises
        scripted
            .and_then(Money::from_json)
            .and_then(Money::finite_value)
            .ok_or_else(|| SpendReadError::Unverifiable {
                counter_key: query.counter_key.to_owned(),
            })
    }
}

// budget_alerts' CallInfo, so the recorded alert is the payload LiteLLM would deliver
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
    projected_spend: Option<f64>,
    event_group: &'static str,
    alert_emails: Option<Vec<String>>,
    max_budget_alert_emails: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Default)]
pub(crate) struct Recorder(Mutex<Vec<Value>>);

impl Recorder {
    pub(crate) fn alerts(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TenantAlertSink for Recorder {
    fn schedule(&self, alert: TenantAlert) {
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
            key_alias: None,
            projected_exceeded_date: None,
            projected_spend: None,
            event_group: event_group(alert.event_group),
            alert_emails: None,
            max_budget_alert_emails: None,
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({"type": kind(alert.kind), "info": info}));
    }
}

// the budget_alerts type each tenant check queues
const fn kind(kind: TenantAlertKind) -> &'static str {
    match kind {
        TenantAlertKind::ProjectBudget => "project_budget",
        TenantAlertKind::OrganizationBudget => "organization_budget",
        TenantAlertKind::SoftBudget => "soft_budget",
    }
}

const fn event_group(group: TenantEventGroup) -> &'static str {
    match group {
        TenantEventGroup::Project => "project",
        TenantEventGroup::Organization => "organization",
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

// LiteLLM's money is a float, so a whole amount still reaches the oracle as 10.0
pub(crate) fn float(value: Decimal) -> Value {
    value
        .to_f64()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

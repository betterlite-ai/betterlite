use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_pricing::project::ProjectModelGrant;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_tenant_budgets::alerts::{TenantAlert, TenantAlertKind, TenantAlertSink};
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_error::proxy_exception::ProxyException;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use secrecy::ExposeSecret as _;
use serde_json::{Number, Value, json};

// the counter values a case scripts, handed out in read order, and the query each read carried
pub(crate) struct ScriptedSpend {
    spends: Vec<JsonData>,
    reads: Mutex<Vec<Value>>,
}

impl ScriptedSpend {
    pub(crate) fn new(spends: &[JsonData]) -> Self {
        Self {
            spends: spends.to_vec(),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reads(&self) -> Vec<Value> {
        match self.reads.lock() {
            Ok(reads) => reads.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait]
impl SpendReader for ScriptedSpend {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        let scripted = self.spends.get(reads.len());
        reads.push(json!({
            "counter_key": query.counter_key,
            "fallback_spend": query
                .fallback_spend
                .to_f64()
                .and_then(Number::from_f64)
                .map_or(Value::Null, Value::Number),
            "max_budget": query.max_budget.map_or(Value::Null, amount_value),
            "fallback_authoritative": query.fallback_authoritative,
        }));
        drop(reads);
        scripted
            .and_then(Money::from_json)
            .and_then(Money::finite_value)
            .ok_or_else(|| SpendReadError::Unverifiable {
                counter_key: query.counter_key.to_owned(),
            })
    }
}

// both project checks queue their alert through budget_alerts; the case compares what that payload carries
#[derive(Default)]
pub(crate) struct ProjectAlerts(Mutex<Vec<Value>>);

impl ProjectAlerts {
    pub(crate) fn alerts(&self) -> Vec<Value> {
        match self.0.lock() {
            Ok(alerts) => alerts.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl TenantAlertSink for ProjectAlerts {
    fn schedule(&self, alert: TenantAlert) {
        let record = json!({
            "type": alert_type(alert.kind),
            "spend": amount_value(alert.spend),
            "max_budget": alert.max_budget.map_or(Value::Null, amount_value),
            "soft_budget": alert.soft_budget.map_or(Value::Null, amount_value),
        });
        match self.0.lock() {
            Ok(mut alerts) => alerts.push(record),
            Err(poisoned) => poisoned.into_inner().push(record),
        }
    }
}

const fn alert_type(kind: TenantAlertKind) -> &'static str {
    match kind {
        TenantAlertKind::ProjectBudget => "project_budget",
        TenantAlertKind::OrganizationBudget => "organization_budget",
        TenantAlertKind::SoftBudget => "soft_budget",
    }
}

// the oracle cases keep the project's model list empty, so the grant never has to reach a router
pub(crate) struct NoGrant;

impl ProjectModelGrant for NoGrant {
    fn check(
        &self,
        _model: &RequestedModels,
        _project_row: &ProjectTable,
        _alias_targets: &BTreeMap<String, String>,
    ) -> Result<(), EntityModelAccessError> {
        Ok(())
    }
}

// BudgetExceededError's caller-visible fields, with money as the float LiteLLM holds
pub(crate) fn denial_value(denial: &Denial) -> Value {
    json!({
        "status": denial.status_code,
        "current_cost": amount_value(denial.current_cost),
        "max_budget": amount_value(denial.max_budget),
        "entity_type": denial.entity_type,
        "entity_id": denial.entity_id.as_ref().map(|entity| entity.expose_secret().to_owned()),
    })
}

// the OpenAI error object LiteLLM's auth handler builds: type, param and code as strings
pub(crate) fn envelope(exception: &ProxyException) -> Value {
    json!({
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
    })
}

pub(crate) fn amount_value(value: Decimal) -> Value {
    value
        .to_f64()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

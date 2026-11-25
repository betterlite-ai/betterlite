use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_key_budgets::alerts::BudgetAlertSink;
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_key_budgets::{Admission, KeyBudgetError, admit, effective_budgets};
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::auth_key_budgets::Settings;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::auth::auth_identity::AuthContext;
use crate::auth::current_spend::spend_failure_exception;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// what the auth service resolved before this stage: the caller's key, the collaborators the checks read, and the clock
pub struct KeyBudgetRule {
    pub auth: Option<UserAPIKeyAuth>,
    pub user_email: Option<String>,
    pub reader: Option<Arc<dyn SpendReader>>,
    pub alerts: Option<Arc<dyn BudgetAlertSink>>,
    pub durations: Option<Arc<dyn DurationParser + Send + Sync>>,
    pub throttle_pct: Option<f64>,
    pub clock: fn() -> DateTime<Utc>,
    pub applied: Mutex<Option<UserAPIKeyAuth>>,
}

impl Default for KeyBudgetRule {
    fn default() -> Self {
        Self {
            auth: None,
            user_email: None,
            reader: None,
            alerts: None,
            durations: None,
            throttle_pct: None,
            clock: Utc::now,
            applied: Mutex::new(None),
        }
    }
}

impl fmt::Debug for KeyBudgetRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyBudgetRule")
            .field("has_auth", &self.auth.is_some())
            .field("user_email", &self.user_email)
            .field("has_reader", &self.reader.is_some())
            .field("has_alerts", &self.alerts.is_some())
            .field("has_durations", &self.durations.is_some())
            .field(
                "has_applied",
                &self.applied.try_lock().is_ok_and(|key| key.is_some()),
            )
            .field("throttle_pct", &self.throttle_pct)
            .finish_non_exhaustive()
    }
}

impl KeyBudgetRule {
    // the key after the checks, which carries any request-local budget_throttle_pct they set
    #[must_use]
    pub fn applied(&self) -> Option<UserAPIKeyAuth> {
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Debug)]
pub struct KeyBudgetStageInput {
    pub context: AuthContext,
    pub rule: Arc<KeyBudgetRule>,
    pub settings: Arc<Settings>,
}

// the authentication service inserts the rule once it resolved the caller's key; without one the stage is inert
pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<KeyBudgetRule> {
    request
        .extensions()
        .get::<Arc<KeyBudgetRule>>()
        .map_or_else(|| Arc::new(KeyBudgetRule::default()), Arc::clone)
}

// a router mounted without the proxy state has no loaded section, so the pinned defaults apply
fn settings_for_request(request: &Request<Body>) -> Arc<Settings> {
    request.extensions().get::<Arc<ServerState>>().map_or_else(
        || Arc::new(Settings::default()),
        |state| Arc::clone(&state.auth_key_budgets.settings),
    )
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<KeyBudgetStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    Ok(KeyBudgetStageInput {
        context,
        rule: rule_for_request(request),
        settings: settings_for_request(request),
    })
}

// the builder runs the alert, hard, soft and window checks in order; a hard denial and an unverifiable read are the failures it reports
pub(crate) async fn prefetch(stage: &KeyBudgetStageInput) -> Result<(), Box<ProxyException>> {
    let Some(auth) = stage.rule.auth.as_ref() else {
        return Ok(());
    };
    let now = (stage.rule.clock)();
    let budgets = effective_budgets(auth, now)
        .map_err(|error| Box::new(authentication_exception(&error.to_string())))?;
    let mut checked = auth.clone();
    let unconfigured = UnconfiguredReader;
    let no_durations = NoDurations;
    let reader: &dyn SpendReader = stage.rule.reader.as_deref().unwrap_or(&unconfigured);
    let durations = stage.rule.durations.as_deref().unwrap_or(&no_durations);
    let admission = Admission {
        reader,
        alerts: stage.rule.alerts.as_deref(),
        durations,
        global_alert_emails: stage.settings.default_alert_emails.as_ref(),
        alert_fraction: Money::from_f64(stage.settings.alert_fraction),
        throttle_pct: stage.rule.throttle_pct,
        now,
        user_email: stage.rule.user_email.as_deref(),
    };
    admit(&mut checked, &budgets, &admission)
        .await
        .map_err(|error| Box::new(key_budget_exception(&error)))?;
    *stage
        .rule
        .applied
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(checked);
    Ok(())
}

fn key_budget_exception(error: &KeyBudgetError) -> ProxyException {
    match error {
        KeyBudgetError::Denied(denial) => budget_exceeded_exception(denial),
        KeyBudgetError::Spend(failure) => spend_failure_exception(failure),
        KeyBudgetError::Window(_) | KeyBudgetError::Alert(_) => {
            authentication_exception(&error.to_string())
        }
    }
}

// BudgetExceededError's own status, which B757 lets a deployment change
fn budget_exceeded_exception(denial: &Denial) -> ProxyException {
    let failure = AuthFailure::BudgetExceeded {
        message: denial.message.clone(),
        status_code: Some(denial.status_code),
        llm_provider: String::new(),
    };
    as_proxy_exception(&failure).exception().clone()
}

fn authentication_exception(message: &str) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: format!("Authentication Error, {message}"),
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Status(401_i64),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

// a rule with budgets but no reader cannot verify spend, which raises the 503; a rule with no budgets never reads
struct UnconfiguredReader;

#[async_trait]
impl SpendReader for UnconfiguredReader {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}

struct NoDurations;

impl DurationParser for NoDurations {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

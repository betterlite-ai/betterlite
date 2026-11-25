#[path = "key_budgets/fixtures/players.rs"]
mod fixtures;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use betterlite_auth::auth_key_budgets::email::merge_alert_email_configs;
use betterlite_auth::auth_key_budgets::expiry::TempBudgetError;
use betterlite_auth::auth_key_budgets::key_budget::Denial;
use betterlite_auth::auth_key_budgets::money::Money;
use betterlite_auth::auth_key_budgets::team_member::{TeamMemberAlert, team_member_alert_check};
use betterlite_auth::auth_key_budgets::temp::apply_temp_budget_increase;
use betterlite_auth::auth_key_budgets::{Admission, KeyBudgetError, admit, effective_budgets};
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::fixtures::{CaseDurations, Recorder, ScriptedSpend, decimal_json, money_json};

// litellm.constants.EMAIL_BUDGET_ALERT_MAX_SPEND_ALERT_PERCENTAGE's default, which the case env leaves alone
const ALERT_FRACTION: f64 = 0.8;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

// serde reads a missing Option field as None, so only the function has to be present
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    token: Option<BTreeMap<String, JsonData>>,
    spends: Option<Vec<JsonData>>,
    now: Option<String>,
    throttle_pct: Option<f64>,
    global_cfg: Option<BTreeMap<String, JsonData>>,
    user_email: Option<String>,
    per_key_cfg: Option<BTreeMap<String, JsonData>>,
    team_id: Option<String>,
    team_alias: Option<String>,
    team_metadata: Option<BTreeMap<String, JsonData>>,
    organization_id: Option<String>,
    user_id: Option<String>,
    spend: Option<JsonData>,
    max_budget: Option<JsonData>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Admit,
    TempUpdate,
    EmailMerge,
    TeamMember,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let answers: Vec<Value> = input
            .calls
            .iter()
            .map(|call| answer(call, &runtime))
            .collect();
        Ok(Value::Array(answers))
    }
}

// LiteLLM's checks run in one event loop per request, so every admit call shares the runtime's thread
fn answer(call: &Call, runtime: &Runtime) -> Value {
    match execute(call, runtime) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(_failure) => json!({"ok": false}),
    }
}

fn execute(call: &Call, runtime: &Runtime) -> Result<Value, Failure> {
    match call.function {
        Function::Admit => runtime.block_on(admit_value(call)),
        Function::TempUpdate => temp_update_value(call),
        // _merge_budget_alert_email_configs: two empty mappings merge to None
        Function::EmailMerge => Ok(json!({
            "merged": merge_alert_email_configs(call.global_cfg.as_ref(), call.per_key_cfg.as_ref())
        })),
        Function::TeamMember => team_member_value(call),
    }
}

// the four LiteLLM key budget checks, in the order user_api_key_auth runs them
async fn admit_value(call: &Call) -> Result<Value, Failure> {
    let auth = auth_of(call)?;
    // user_api_key_auth.datetime, held at the moment the case fixes
    let clock = call.now.as_deref().ok_or(Failure::Missing("the clock"))?;
    let now = DateTime::parse_from_rfc3339(clock)?.with_timezone(&Utc);
    let reader = scripted_spend(call)?;
    let recorder = Recorder::default();
    let budgets = effective_budgets(&auth, now)?;
    let durations = CaseDurations(TimeZoneDatabase::from_env());
    let admission = Admission {
        reader: &reader,
        alerts: Some(&recorder),
        durations: &durations,
        global_alert_emails: call.global_cfg.as_ref(),
        alert_fraction: Money::from_f64(ALERT_FRACTION),
        throttle_pct: call.throttle_pct,
        now,
        user_email: call.user_email.as_deref(),
    };
    let mut checked = auth;
    let denial = match admit(&mut checked, &budgets, &admission).await {
        Ok(()) => Value::Null,
        Err(KeyBudgetError::Denied(denial)) => denial_json(&denial),
        Err(error) => return Err(Failure::Budget(error)),
    };
    let throttle_pct = checked.request_quota_view().throttle_pct;
    Ok(json!({
        "alerts": recorder.take(),
        "reads": reader.take(),
        "throttle_pct": throttle_pct,
        "fraction": ALERT_FRACTION,
        "denial": denial,
    }))
}

// _update_key_budget_with_temp_budget_increase: the copy keeps the original maximum for an inactive increase
fn temp_update_value(call: &Call) -> Result<Value, Failure> {
    let auth = auth_of(call)?;
    let clock = call.now.as_deref().ok_or(Failure::Missing("the clock"))?;
    let now = DateTime::parse_from_rfc3339(clock)?.with_timezone(&Utc);
    let update = apply_temp_budget_increase(&auth, now)?;
    Ok(json!({"max_budget": money_json(update.max_budget)}))
}

// _team_member_max_budget_alert_check: a team's own map applies with no global merge
fn team_member_value(call: &Call) -> Result<Value, Failure> {
    let alert = TeamMemberAlert {
        team_id: call
            .team_id
            .as_deref()
            .ok_or(Failure::Missing("the team id"))?,
        team_alias: call.team_alias.as_deref(),
        team_metadata: call.team_metadata.as_ref(),
        organization_id: call.organization_id.as_deref(),
        user_id: call
            .user_id
            .as_deref()
            .ok_or(Failure::Missing("the user id"))?,
        user_email: call.user_email.as_deref(),
        spend: Money::from_json(call.spend.as_ref().ok_or(Failure::Missing("the spend"))?)
            .and_then(Money::finite_value)
            .ok_or(Failure::Missing("a finite spend"))?,
        max_budget: Money::from_json(
            call.max_budget
                .as_ref()
                .ok_or(Failure::Missing("the max budget"))?,
        )
        .ok_or(Failure::Missing("the max budget"))?,
    };
    let recorder = Recorder::default();
    team_member_alert_check(&alert, Some(&recorder));
    Ok(json!({"alerts": recorder.take()}))
}

// the reads the case scripts, consumed in order as the checks ask for them
fn scripted_spend(call: &Call) -> Result<ScriptedSpend, Failure> {
    let spends = call
        .spends
        .as_deref()
        .ok_or(Failure::Missing("the spend reads"))?;
    Ok(ScriptedSpend {
        spends: spends.to_vec(),
        next: AtomicUsize::new(0),
        reads: Mutex::new(Vec::new()),
    })
}

fn auth_of(call: &Call) -> Result<UserAPIKeyAuth, Failure> {
    let fields = call
        .token
        .clone()
        .ok_or(Failure::Missing("the token fields"))?;
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(Failure::Auth)
}

// BudgetExceededError's caller-visible fields, with the status litellm.budget_exceeded_status_code defaults to
fn denial_json(denial: &Denial) -> Value {
    let entity_id = denial.entity_id.as_ref().map_or(Value::Null, |entity| {
        Value::String(entity.expose_secret().to_owned())
    });
    json!({
        "status": denial.status_code,
        "current_cost": decimal_json(denial.current_cost),
        "max_budget": decimal_json(denial.max_budget),
        "entity_type": denial.entity_type,
        "entity_id": entity_id,
    })
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Temp(#[from] TempBudgetError),
    #[error(transparent)]
    Clock(#[from] chrono::ParseError),
    #[error(transparent)]
    Budget(#[from] KeyBudgetError),
    #[error("the call does not name {0}")]
    Missing(&'static str),
}

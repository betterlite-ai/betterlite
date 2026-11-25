#[path = "team_budget_admission/fixtures/collaborators.rs"]
mod fixtures;
mod tests;

use std::collections::BTreeMap;
use std::sync::PoisonError;

use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_auth::team_budget_admission::error::TeamBudgetError;
use betterlite_auth::team_budget_admission::soft::{SoftOutcome, team_soft_budget_check};
use betterlite_auth::team_budget_admission::sources::Membership;
use betterlite_auth::team_budget_admission::{TeamAdmission, TeamRequest, admit};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::fixtures::{CaseDurations, Recorder, Seeded, denial_json, exception_json};

// litellm.constants.DEFAULT_MAX_RECURSE_DEPTH, which the case env leaves alone
const MAX_RECURSE_DEPTH: i64 = 100;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

// the membership is always handed over loaded, as common_checks does, so no call reaches a membership store
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: Function,
    team: Option<LiteLLMTeamTable>,
    token: Option<BTreeMap<String, JsonData>>,
    membership: Option<LiteLLMTeamMembership>,
    default_budget: Option<LiteLLMBudgetTable>,
    user_email: Option<String>,
    model: Option<Model>,
    spends: Vec<JsonData>,
    now: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Admit,
    Soft,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Model {
    One(String),
    Many(Vec<String>),
}

impl Model {
    fn requested(&self) -> RequestedModels {
        match self {
            Self::One(name) => RequestedModels::One(name.clone()),
            Self::Many(names) => RequestedModels::Many(names.clone()),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut answers = Vec::new();
        for call in &input.calls {
            answers.push(match runtime.block_on(answer(call)) {
                Ok(value) => json!({"ok": true, "value": value}),
                Err(_failure) => json!({"ok": false}),
            });
        }
        Ok(Value::Array(answers))
    }
}

async fn answer(call: &Call) -> Result<Value, Failure> {
    let token = match call.token.clone() {
        Some(fields) => Some(UserAPIKeyAuth::construct(AuthInput::Dict(fields))?),
        None => None,
    };
    let Function::Soft = call.function else {
        return admit_value(call, token.as_ref()).await;
    };
    let recorder = Recorder::default();
    let outcome = team_soft_budget_check(call.team.as_ref(), token.as_ref(), Some(&recorder));
    let logs = soft_logs(outcome, call.team.as_ref());
    let alerts = recorder
        .0
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(json!({"alerts": alerts, "logs": logs}))
}

// common_checks' member model scope, then the team and member budgets, at the case's clock
async fn admit_value(call: &Call, token: Option<&UserAPIKeyAuth>) -> Result<Value, Failure> {
    let recorder = Recorder::default();
    let seeded = Seeded::new(&call.spends, call.default_budget.clone());
    let model = call
        .model
        .as_ref()
        .map_or(RequestedModels::None, Model::requested);
    let aliases = BTreeMap::new();
    let models = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &UnresolvedProviderInference,
        max_recurse_depth: MAX_RECURSE_DEPTH,
    };
    let admission = TeamAdmission {
        reader: &seeded,
        memberships: &seeded,
        defaults: &seeded,
        member_alerts: Some(&recorder),
        team_alerts: Some(&recorder),
        durations: &CaseDurations(TimeZoneDatabase::from_env()),
        models: &models,
        now: DateTime::parse_from_rfc3339(&call.now)?.with_timezone(&Utc),
    };
    let request = TeamRequest {
        team: call.team.as_ref(),
        token,
        user_email: call.user_email.as_deref(),
        model: &model,
        membership: Membership::Loaded(call.membership.clone().map(Box::new)),
    };
    let (denial, model_denial) = match admit(&admission, &request).await {
        Ok(_outcome) => (Value::Null, Value::Null),
        Err(TeamBudgetError::Denied(denial)) => (denial_json(&denial), Value::Null),
        Err(TeamBudgetError::ModelDenied(denied)) => {
            (Value::Null, exception_json(denied.as_proxy_exception()))
        }
        Err(
            error @ (TeamBudgetError::Spend(_)
            | TeamBudgetError::Membership(_)
            | TeamBudgetError::DefaultBudget(_)
            | TeamBudgetError::RecurseDepth { .. }),
        ) => return Err(Failure::Admission(error)),
    };
    Ok(json!({
        "reads": seeded.reads.into_inner().unwrap_or_else(PoisonError::into_inner),
        "alerts": recorder.0.into_inner().unwrap_or_else(PoisonError::into_inner),
        "denial": denial,
        "model_denial": model_denial,
    }))
}

// verbose_proxy_logger.debug's text in _team_soft_budget_check, with Python's float repr of the row's amounts
fn soft_logs(outcome: SoftOutcome, team: Option<&LiteLLMTeamTable>) -> Vec<String> {
    let Some(row) = team else {
        return Vec::new();
    };
    let team_id = &row.fields.base.team_id;
    let spend = row.fields.spend.and_then(|value| value.to_f64());
    let soft_budget = row.fields.base.soft_budget.and_then(|value| value.to_f64());
    let crossed = format!(
        "Crossed Soft Budget for team {team_id}, spend {:?}, soft_budget {:?}",
        spend.unwrap_or(f64::NAN),
        soft_budget.unwrap_or(f64::NAN)
    );
    let skipped = format!(
        "Skipping team soft budget alert for team {team_id}: no alert_emails configured in metadata.soft_budget_alerting_emails"
    );
    match outcome {
        SoftOutcome::NotCrossed => Vec::new(),
        SoftOutcome::CrossedWithoutToken | SoftOutcome::Alerted => vec![crossed],
        SoftOutcome::SkippedWithoutRecipients => vec![crossed, skipped],
    }
}

#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Clock(#[from] chrono::ParseError),
    #[error(transparent)]
    Admission(TeamBudgetError),
}

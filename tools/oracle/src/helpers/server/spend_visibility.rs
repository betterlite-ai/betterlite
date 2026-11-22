use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::spend::visibility::error::VisibilityError;
use betterlite_server::spend::visibility::service::SpendLogVisibility;
use betterlite_server::team_access::caller::TeamCaller;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

use self::fakes::{CaseStore, RecordedRead, case_store, user_lookup};

mod fakes;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    caller: CallerInput,
    lookup_id: String,
    users: Vec<UserInput>,
    teams: Vec<JsonData>,
    fail_teams: bool,
    owners: Vec<OwnerInput>,
    rows: Vec<RowInput>,
    payload: Option<JsonData>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerInput {
    user_id: Option<String>,
    user_role: Option<UserRole>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserInput {
    user_id: String,
    teams: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerInput {
    user: Option<String>,
    team_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowInput {
    request_id: String,
    user: Option<String>,
    team_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct StepAnswer {
    reads: Vec<RecordedRead>,
    storage_key: Option<String>,
    outcome: &'static str,
    status: Option<u16>,
    detail: Option<Detail>,
}

#[derive(Debug, Serialize)]
struct Detail {
    error: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut answers = Vec::new();
        for step in &input.steps {
            answers.push(runtime.block_on(run_step(step))?);
        }
        Ok(serde_json::to_value(answers)?)
    }
}

async fn run_step(step: &Step) -> Result<StepAnswer, Box<HelperError>> {
    let store = Arc::new(case_store(step)?);
    let checks = SpendLogVisibility {
        store: Arc::<CaseStore>::clone(&store),
        users: Arc::new(user_lookup(&step.users)?),
    };
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_value(serde_json::to_value(&step.caller)?)?;
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let caller = TeamCaller::try_from(&auth).or_value_error()?;
    let mut storage_key = None;
    let viewed = view(&checks, &caller, step, &mut storage_key).await;
    let reads = store
        .reads
        .lock()
        .await
        .drain(..)
        .collect::<Result<Vec<RecordedRead>, _>>()?;
    let (outcome, status, detail) = classify(viewed);
    Ok(StepAnswer {
        reads,
        storage_key,
        outcome,
        status,
        detail,
    })
}

// the step's answer: a served row or payload, the source's 403, or a read failure the source never handles
fn classify(
    viewed: Result<&'static str, VisibilityError>,
) -> (&'static str, Option<u16>, Option<Detail>) {
    match viewed {
        Ok(served) => (served, None, None),
        Err(denied @ VisibilityError::Forbidden { .. }) => (
            "http",
            Some(403_u16),
            Some(Detail {
                error: denied.to_string(),
            }),
        ),
        Err(
            VisibilityError::Database(_)
            | VisibilityError::Team(_)
            | VisibilityError::TeamList(_)
            | VisibilityError::TeamRecord(_)
            | VisibilityError::Users(_),
        ) => ("error", None, None),
    }
}

// the order ui_view_request_response_for_request_id runs the checks in, with one cold-storage logger
async fn view(
    checks: &SpendLogVisibility,
    caller: &TeamCaller,
    step: &Step,
    storage_key: &mut Option<String>,
) -> Result<&'static str, VisibilityError> {
    checks
        .assert_can_view_request_id(caller, &step.lookup_id)
        .await?;
    let resolved = checks.resolve_payload_row(caller, &step.lookup_id).await?;
    *storage_key = Some(resolved.storage_key);
    let Some(payload) = &step.payload else {
        return Ok(if resolved.row.is_some() {
            "row"
        } else {
            "none"
        });
    };
    checks
        .assert_owns_cold_payload(caller, payload, &step.lookup_id)
        .await?;
    Ok("payload")
}

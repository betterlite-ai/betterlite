use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use betterlite_server::model_write_fields::access::{
    ModelCall, ModelWriteAccess, can_make_team_model_call,
};
use betterlite_server::model_write_fields::error::{
    Detail, MemberCallError, ModelWriteError, Rejection,
};
use betterlite_server::model_write_fields::fields::{
    ModelPatch, can_attach_credential, can_set_session_tags,
};
use betterlite_server::model_write_fields::member::MemberWrite;
use betterlite_server::team_access::access::TeamAccess;
use betterlite_server::team_access::caller::TeamCaller;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use tokio::sync::Mutex;

use self::fakes::{CaseTeams, NoOrgAdmins, Unregistered};

mod fakes;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    function: Function,
    caller: CallerInput,
    team_id: Option<String>,
    teams: Vec<LiteLLMTeamTable>,
    allow_missing_team: bool,
    patch: Option<ModelPatch>,
    stored: Option<ModelPatch>,
    null_detaches: bool,
    salt_key: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    ModelCall,
    CreateWrite,
    UpdateWrite,
    TeamAction,
    TeamCall,
    AttachCredential,
    SessionTags,
}

// the UserAPIKeyAuth fields the case sets, built into a real key so the production conversion runs
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerInput {
    user_id: Option<String>,
    user_role: Option<UserRole>,
    team_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct StepResult {
    value: Option<bool>,
    error: Option<ErrorOut>,
    team_reads: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Shape {
    Message,
    Error,
}

#[derive(Debug, Serialize)]
struct ErrorOut {
    status: Option<u16>,
    detail: Option<Shape>,
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut results = Vec::new();
        for step in &input.steps {
            results.push(runtime.block_on(run_step(step))?);
        }
        Ok(serde_json::to_value(results)?)
    }
}

async fn run_step(step: &Step) -> Result<StepResult, Box<HelperError>> {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_value(serde_json::to_value(&step.caller)?)?;
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let caller = TeamCaller::try_from(&auth).or_value_error()?;
    let teams = CaseTeams {
        teams: step.teams.clone(),
        reads: Mutex::new(Vec::new()),
    };
    let outcome = answer(step, &caller, &teams).await?;
    let team_reads = teams.reads.lock().await.clone();
    Ok(match outcome {
        Ok(()) => StepResult {
            value: Some(true),
            error: None,
            team_reads,
        },
        Err(error) => StepResult {
            value: None,
            error: Some(error_out(&error)),
            team_reads,
        },
    })
}

// the status, detail shape, type, param and code LiteLLM's raised exception carries; the message is betterlite's own
fn error_out(error: &ModelWriteError) -> ErrorOut {
    let rejection = error.rejection();
    let status = rejection.as_ref().map(|found| found.status.as_u16());
    let (detail, exception) = match rejection.map(|Rejection { detail, .. }| detail) {
        Some(Detail::Message(_)) => (Some(Shape::Message), None),
        Some(Detail::Error(_)) => (Some(Shape::Error), None),
        Some(Detail::Proxy(raised)) => (None, Some(raised)),
        None => (None, None),
    };
    ErrorOut {
        status,
        detail,
        error_type: exception.as_ref().map(|raised| raised.error_type.clone()),
        param: exception.as_ref().and_then(|raised| raised.param.clone()),
        code: exception.map(|raised| raised.code),
    }
}

async fn answer(
    step: &Step,
    caller: &TeamCaller,
    teams: &CaseTeams,
) -> Result<Result<(), ModelWriteError>, Box<HelperError>> {
    let team_access = TeamAccess {
        org_roles: Arc::new(NoOrgAdmins),
    };
    let writes = ModelWriteAccess {
        teams,
        team_access: &team_access,
    };
    let team_id = step.team_id.as_deref();
    Ok(match step.function {
        Function::ModelCall => model_call(&writes, step, caller).await,
        Function::CreateWrite => {
            let write = MemberWrite::Create { deployment: &() };
            member_write(&writes, caller, team_id, write).await
        }
        Function::UpdateWrite => {
            let write = MemberWrite::Update {
                patch: &(),
                existing: &(),
            };
            member_write(&writes, caller, team_id, write).await
        }
        Function::TeamAction => {
            writes
                .can_make_model_call(caller, team_id, ModelCall::TeamAction)
                .await
        }
        Function::TeamCall => team_call(step, caller)?,
        Function::AttachCredential => attach_credential(step, caller),
        Function::SessionTags => {
            can_set_session_tags(step.patch.as_ref(), step.stored.as_ref(), caller)
        }
    })
}

async fn model_call(
    writes: &ModelWriteAccess<'_>,
    step: &Step,
    caller: &TeamCaller,
) -> Result<(), ModelWriteError> {
    let call = if step.allow_missing_team {
        ModelCall::Delete
    } else {
        ModelCall::Write
    };
    writes
        .can_make_model_call(caller, step.team_id.as_deref(), call)
        .await
}

// team_obj is the case's team row with that id, or None when the case holds none
fn team_call(
    step: &Step,
    caller: &TeamCaller,
) -> Result<Result<(), ModelWriteError>, Box<HelperError>> {
    let owner = step
        .team_id
        .as_deref()
        .ok_or("a team_call step names its team")
        .or_value_error()?;
    let found = step
        .teams
        .iter()
        .find(|stored| stored.fields.base.team_id == owner);
    Ok(can_make_team_model_call(
        owner,
        caller,
        found.map(|known| &known.fields.base),
    ))
}

// the salt key stands in for LITELLM_SALT_KEY, which the Python side sets for the same step
fn attach_credential(step: &Step, caller: &TeamCaller) -> Result<(), ModelWriteError> {
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(step.salt_key.clone())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    can_attach_credential(
        step.patch.as_ref(),
        step.stored.as_ref(),
        caller,
        step.null_detaches,
        &cipher,
    )
}

async fn member_write(
    writes: &ModelWriteAccess<'_>,
    caller: &TeamCaller,
    team_id: Option<&str>,
    write: MemberWrite<'_, (), ()>,
) -> Result<(), ModelWriteError> {
    match writes
        .can_make_member_write(caller, team_id, None::<&Unregistered>, write)
        .await
    {
        Ok(_grant) => Ok(()),
        Err(MemberCallError::Access(error)) => Err(error),
        Err(MemberCallError::Refused(never)) => match never {},
    }
}

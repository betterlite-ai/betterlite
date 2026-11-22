mod fakes;
mod inputs;
mod values;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::memory_records::access::assert_write_access;
use betterlite_server::memory_records::caller::{MemoryCaller, is_admin};
use betterlite_server::memory_records::error::{MemoryHttpError, internal_error};
use betterlite_server::memory_records::filter::{key_filter, visibility_filter};
use betterlite_server::memory_records::scope::resolve_scope;
use betterlite_server::memory_records::unique::is_unique_violation;
use betterlite_server::team_access::access::TeamAccess;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::runtime::Builder;

use self::fakes::{NoOrgRoles, RefusingTeams, case_row};
use self::inputs::{CallerInput, CaseError, InternalErrorValue, RowInput};
use self::values::{model_value, request_value};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

// one memory helper call per step, every field from LiteLLM's own signature, so a case says exactly what the behavior reads
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    function: Function,
    caller: CallerInput,
    row: RowInput,
    request: Option<RequestKind>,
    model: Option<ModelKind>,
    body: JsonData,
    search: Option<String>,
    key_prefix: Option<String>,
    key: Option<String>,
    requested_user_id: Option<String>,
    requested_team_id: Option<String>,
    code: Option<String>,
    message: String,
    log_message: String,
    detail: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Visibility,
    IsAdmin,
    KeyFilter,
    Scope,
    WriteAccess,
    UniqueViolation,
    InternalError,
    Request,
    Model,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RequestKind {
    Create,
    Update,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModelKind {
    Row,
    List,
    Delete,
}

#[derive(Debug, Serialize)]
struct StepResult {
    value: Value,
    error: Option<ErrorOut>,
}

#[derive(Debug, Serialize)]
struct ErrorOut {
    status: Option<u16>,
    detail: Option<String>,
    #[serde(rename = "type")]
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

// a step's failure: an HTTP error the helper raises, or input LiteLLM's own model rejects
enum StepFailure {
    Http(MemoryHttpError),
    Input,
}

impl From<MemoryHttpError> for StepFailure {
    fn from(error: MemoryHttpError) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for StepFailure {
    fn from(_error: serde_json::Error) -> Self {
        Self::Input
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(runtime.block_on(run_step(step))?);
        }
        Ok(serde_json::to_value(results)?)
    }
}

async fn run_step(step: &Step) -> Result<StepResult, Box<HelperError>> {
    let caller = build_caller(&step.caller)?;
    let outcome = answer(step, &caller).await;
    Ok(match outcome {
        Ok(value) => StepResult { value, error: None },
        Err(failure) => StepResult {
            value: Value::Null,
            error: Some(error_out(&failure)),
        },
    })
}

fn build_caller(caller: &CallerInput) -> Result<MemoryCaller, Box<HelperError>> {
    let fields: BTreeMap<String, JsonData> = serde_json::from_value(serde_json::to_value(caller)?)?;
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    MemoryCaller::try_from(&key).or_value_error()
}

fn error_out(failure: &StepFailure) -> ErrorOut {
    match failure {
        StepFailure::Http(error) => ErrorOut {
            status: Some(error.status.as_u16()),
            detail: Some(error.detail.clone()),
            error_type: None,
            param: None,
            code: None,
        },
        StepFailure::Input => ErrorOut {
            status: None,
            detail: None,
            error_type: None,
            param: None,
            code: None,
        },
    }
}

async fn answer(step: &Step, caller: &MemoryCaller) -> Result<Value, StepFailure> {
    match step.function {
        Function::Visibility => Ok(serde_json::to_value(visibility_filter(caller))?),
        Function::IsAdmin => Ok(serde_json::to_value(is_admin(caller))?),
        Function::KeyFilter => Ok(serde_json::to_value(key_filter(
            step.search.as_deref(),
            step.key_prefix.as_deref(),
            step.key.as_deref(),
        ))?),
        Function::Scope => Ok(serde_json::to_value(resolve_scope(
            caller,
            step.requested_user_id.as_deref(),
            step.requested_team_id.as_deref(),
        )?)?),
        Function::WriteAccess => {
            let row = case_row(&step.row);
            let access = TeamAccess {
                org_roles: Arc::new(NoOrgRoles),
            };
            assert_write_access(caller, &row, &RefusingTeams, &access).await?;
            Ok(Value::Null)
        }
        Function::UniqueViolation => Ok(serde_json::to_value(is_unique_violation(
            step.code.as_deref(),
            &step.message,
        ))?),
        Function::InternalError => {
            let source = CaseError {
                message: step.message.clone(),
            };
            let error = internal_error(&step.log_message, &source, &step.detail);
            let value = InternalErrorValue {
                status: error.status.as_u16(),
                detail: error.detail,
            };
            Ok(serde_json::to_value(value)?)
        }
        Function::Request => request_value(step),
        Function::Model => model_value(step),
    }
}

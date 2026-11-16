use std::time::Duration;

use betterlite_anthropic::skills::wire::ListSkillsResponse;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_core::skills::operations::{
    HostedSkillOps, HostedSkillOpsError, SkillTarget, SkillUpload,
};
use betterlite_core::skills::store::repository::SkillRepository;
use betterlite_core::skills::store::{
    DEFAULT_PAGE_LIMIT, DEFAULT_PAGE_OFFSET, SkillPage, StoreError,
};
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value, json};

use super::{PrincipalFields, Step, creation_of, decoded_files, invalid_input, upload_of};

const FIXTURE_MODEL: &str = "fixture-model";
const FIXTURE_CALL_ID: &str = "call-1";
const FIXTURE_TRACE_ID: &str = "trace-1";
const FIXTURE_FUNCTION_ID: &str = "fn-1";
const FIXTURE_START: &str = "2026-01-01T00:00:00+00:00";

enum Call<'request> {
    Create,
    List,
    Get(&'request str),
    Delete(&'request str),
}

impl Call<'_> {
    async fn answer<R: SkillRepository>(
        self,
        ops: &HostedSkillOps<R>,
        step: &Step,
        caller: Option<&Principal>,
        logging: &mut LoggingState,
    ) -> crate::helpers::HelperResult {
        let call_id = step.litellm_call_id.as_deref();
        match self {
            Self::Create => {
                let (decoded, content) = decoded_files(step)?;
                let uploads: Vec<SkillUpload<'_>> = decoded.iter().map(upload_of).collect();
                let request = creation_of(step, caller, &uploads, content.as_deref());
                let outcome = ops.create_skill(logging, request).await;
                Ok(record_answer("create", outcome))
            }
            Self::List => Ok(page_answer(
                ops.list_skills(logging, page_of(step), caller, call_id)
                    .await,
            )),
            Self::Get(skill_id) => {
                let target = SkillTarget {
                    logging,
                    id: skill_id,
                    principal: caller,
                    call_id,
                };
                Ok(record_answer("get", ops.get_skill(target).await))
            }
            Self::Delete(skill_id) => {
                let target = SkillTarget {
                    logging,
                    id: skill_id,
                    principal: caller,
                    call_id,
                };
                Ok(record_answer("delete", ops.delete_skill(target).await))
            }
        }
    }
}

// LiteLLM raises for an unknown action or a missing skill id before it touches the call context
fn call_of(step: &Step) -> Option<Call<'_>> {
    match step.action.as_str() {
        "create" => Some(Call::Create),
        "list" => Some(Call::List),
        "get" => step.skill_id.as_deref().map(Call::Get),
        "delete" => step.skill_id.as_deref().map(Call::Delete),
        _ => None,
    }
}

fn page_of(step: &Step) -> SkillPage {
    SkillPage {
        limit: step.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
        offset: step.offset.unwrap_or(DEFAULT_PAGE_OFFSET),
    }
}

const fn token(error: &HostedSkillOpsError) -> &'static str {
    match error {
        HostedSkillOpsError::Store(StoreError::NotFound { .. }) => "not_found",
        HostedSkillOpsError::Store(StoreError::MissingIdentity) => "missing_identity",
        HostedSkillOpsError::Store(StoreError::NoDatabase) => "no_database",
        HostedSkillOpsError::Store(
            StoreError::Repository { .. } | StoreError::Metadata(_) | StoreError::Cache(_),
        ) => "exception",
        HostedSkillOpsError::Context(_) | HostedSkillOpsError::Timestamp(_) => "exception",
    }
}

fn principal_of(fields: &PrincipalFields) -> Result<Principal, Box<crate::helpers::HelperError>> {
    let mut teams = Vec::new();
    if let Some(id) = &fields.team_id {
        teams.push(json!({"id": id}));
    }
    let roles = if fields.admin {
        json!(["platform_admin"])
    } else {
        json!([])
    };
    let value = json!({
        "principal_type": "human",
        "subject": fields.user_id.as_deref().unwrap_or("scripted"),
        "user": fields.user_id.as_ref().map(|id| json!({"id": id})),
        "organization": fields.org_id.as_ref().map(|id| json!({"id": id})),
        "teams": teams,
        "roles": roles,
        "auth_method": "api_key",
        "credential_ref": {"key_id": &fields.api_key, "token_id": &fields.token},
    });
    serde_json::from_value(value).map_err(|error| invalid_input(error.to_string()))
}

fn logging_state(action: &str) -> Result<LoggingState, Box<crate::helpers::HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| invalid_input(error.to_string()))?;
    let mut correlation = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: FIXTURE_MODEL.to_owned(),
            call_type: action.to_owned(),
            start_time: chrono::DateTime::parse_from_rfc3339(FIXTURE_START)
                .map_err(|error| invalid_input(error.to_string()))?,
            litellm_call_id: FIXTURE_CALL_ID.to_owned(),
            function_id: FIXTURE_FUNCTION_ID.to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some(FIXTURE_TRACE_ID.to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(|error| invalid_input(error.to_string()))
}

fn logging_view(logging: &LoggingState) -> Value {
    let recorded = |name: &str| {
        logging
            .model_call_details
            .get(name)
            .cloned()
            .unwrap_or(Json::Null)
    };
    json!({
        "model": recorded("model"),
        "optional_params": recorded("optional_params"),
        "litellm_params": recorded("litellm_params"),
        "custom_llm_provider": recorded("custom_llm_provider"),
        "litellm_call_id": recorded("litellm_call_id"),
    })
}

fn with_logging(
    mut answer: Value,
    logging: &LoggingState,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let Value::Object(fields) = &mut answer else {
        return Err(invalid_input("a step answer must be an object".to_owned()));
    };
    fields.insert("logging".to_owned(), logging_view(logging));
    Ok(answer)
}

pub(super) async fn run_steps<R>(
    ops: &HostedSkillOps<R>,
    steps: &[Step],
) -> Result<Vec<Value>, Box<crate::helpers::HelperError>>
where
    R: SkillRepository,
{
    let mut answers = Vec::with_capacity(steps.len());
    for step in steps {
        answers.push(run_step(ops, step).await?);
    }
    Ok(answers)
}

async fn run_step<R>(
    ops: &HostedSkillOps<R>,
    step: &Step,
) -> Result<Value, Box<crate::helpers::HelperError>>
where
    R: SkillRepository,
{
    let action = step.action.as_str();
    let Some(call) = call_of(step) else {
        return Ok(json!({"action": action, "error": "invalid_input"}));
    };
    let caller = step.principal.as_ref().map(principal_of).transpose()?;
    let mut logging = logging_state(action)?;
    let answer = call
        .answer(ops, step, caller.as_ref(), &mut logging)
        .await?;
    with_logging(answer, &logging)
}

fn page_answer(outcome: Result<ListSkillsResponse, HostedSkillOpsError>) -> Value {
    match outcome {
        Ok(response) => json!({
            "action": "list",
            "data": response.data,
            "next_page": response.next_page,
            "has_more": response.has_more,
        }),
        Err(error) => json!({"action": "list", "error": token(&error)}),
    }
}

fn record_answer<T>(action: &str, outcome: Result<T, HostedSkillOpsError>) -> Value
where
    T: Serialize,
{
    match outcome {
        Ok(record) => json!({"action": action, "record": record}),
        Err(error) => json!({"action": action, "error": token(&error)}),
    }
}

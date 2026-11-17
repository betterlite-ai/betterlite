use std::error::Error as StdError;
use std::sync::{Mutex, PoisonError};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::bedrock_settings::{BedrockError, error_response_text};
use betterlite_bedrock::converse_response::environment::{
    get_error_class, should_fake_stream, validate_environment,
};
use betterlite_bedrock::converse_response::request::{OptionalParams, RawReply, ReplyRequest};
use betterlite_bedrock::converse_response::transform::{
    TransformCall, TransformError, transform_response,
};
use betterlite_bedrock::converse_response::wire::ModelResponse;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use self::support::{
    TokenizerCounter, environment_setup, failure, fixture, instant, log_view, status_failure,
};

mod support;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Transform(Vec<ReplyCase>),
    FakeStream(Vec<FakeStreamCase>),
    Environment(Vec<EnvironmentCase>),
    ErrorClass(Vec<ErrorClassCase>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReplyCase {
    body: String,
    headers: Vec<(String, String)>,
    model: String,
    messages: Json,
    data: IndexMap<String, Json>,
    #[serde(default)]
    optional_params: OptionalParams,
    json_mode: Option<bool>,
    supplied: SuppliedFields,
    #[serde(default)]
    ids: Vec<String>,
    now: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SuppliedFields {
    id: String,
    model: String,
    object: String,
    #[serde(default)]
    system_fingerprint: Option<String>,
    #[serde(default)]
    service_tier: Option<Json>,
    #[serde(default)]
    trace: Option<Json>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FakeStreamCase {
    model: Option<String>,
    stream: Option<bool>,
    fake_stream: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnvironmentCase {
    #[serde(default)]
    fields: Vec<String>,
    headers: Vec<(String, String)>,
    #[serde(default)]
    litellm_params: Value,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorClassCase {
    message: String,
    status: u16,
    headers: Vec<(String, String)>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Operation;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Operation::Transform(cases) => collect(cases, transform_case),
            Operation::FakeStream(cases) => Ok(json!(
                cases
                    .iter()
                    .map(|case| should_fake_stream(
                        case.model.as_deref(),
                        case.stream,
                        case.fake_stream
                    ))
                    .collect::<Vec<_>>()
            )),
            Operation::Environment(cases) => collect(cases, environment_case),
            Operation::ErrorClass(cases) => {
                let errors = cases
                    .iter()
                    .map(|case| {
                        get_error_class(&case.message, case.status, &case.headers).map_err(failure)
                    })
                    .collect::<Result<Vec<_>, Box<HelperError>>>()?;
                Ok(json!(errors.iter().map(error_view).collect::<Vec<_>>()))
            }
        }
    }
}

fn collect<Case, Output>(
    cases: &[Case],
    one: fn(&Case) -> Result<Output, Box<HelperError>>,
) -> HelperResult
where
    Output: serde::Serialize,
{
    let outputs = cases
        .iter()
        .map(one)
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    serde_json::to_value(outputs).map_err(failure)
}

struct Prepared<'case> {
    request: ReplyRequest<'case>,
    reply: RawReply<'case>,
    ids: FixedIds,
}

fn prepare<'case>(
    case: &'case ReplyCase,
    mappings: &'case IndexMap<String, String>,
) -> Result<Prepared<'case>, Box<HelperError>> {
    Ok(Prepared {
        request: ReplyRequest {
            model: &case.model,
            messages: &case.messages,
            data: RawBody::Object(case.data.clone()),
            json_mode: case.json_mode,
            optional: &case.optional_params,
            tool_name_mappings: mappings,
        },
        reply: RawReply {
            headers: &case.headers,
            body: case.body.as_bytes(),
        },
        ids: FixedIds {
            values: Mutex::new(parse_ids(&case.ids)?),
        },
    })
}

fn transform_case(case: &ReplyCase) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    let (mut state, env) = fixture(&redactor)?;
    let mappings: IndexMap<String, String> = IndexMap::new();
    let Prepared {
        request,
        reply,
        ids,
    } = prepare(case, &mappings)?;
    let result = transform_response(
        TransformCall {
            request,
            log: LogContext {
                state: &mut state,
                env: &env,
            },
            now: instant(case.now)?,
        },
        &reply,
        supplied_response(&case.supplied),
        &ids,
        &TokenizerCounter,
    );
    Ok(match result {
        Ok(response) => json!({
            "response": serde_json::to_value(response).map_err(failure)?,
            "log": log_view(&state),
        }),
        Err(error) => json!({
            "failure": failure_view(&error),
            "log": log_view(&state),
        }),
    })
}

fn supplied_response(fields: &SuppliedFields) -> ModelResponse {
    ModelResponse {
        id: fields.id.clone(),
        created: 0,
        model: fields.model.clone(),
        object: fields.object.clone(),
        system_fingerprint: fields.system_fingerprint.clone(),
        choices: Vec::new(),
        usage: None,
        service_tier: fields.service_tier.clone(),
        trace: fields.trace.clone(),
    }
}

// LiteLLM raises a BedrockError only for a body it cannot read as a response block; every later missing member keeps its own exception.
fn failure_view<CounterFailure>(error: &TransformError<CounterFailure>) -> Value
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    let headers = match error {
        TransformError::Conversion {
            error: bedrock_error,
            ..
        } => json!(bedrock_error.response_headers),
        TransformError::Header(_)
        | TransformError::Clock(_)
        | TransformError::Reply(_)
        | TransformError::Message(_)
        | TransformError::TokenUsage(_)
        | TransformError::Usage(_) => Value::Null,
    };
    json!({"status": error.status(), "headers": headers})
}

struct FixedIds {
    values: Mutex<Vec<Uuid>>,
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(0)
    }
}

fn parse_ids(values: &[String]) -> Result<Vec<Uuid>, Box<HelperError>> {
    values
        .iter()
        .map(|value| Uuid::parse_str(value).or_value_error())
        .collect()
}

fn environment_case(case: &EnvironmentCase) -> Result<Value, Box<HelperError>> {
    let setup = environment_setup(
        &case.fields,
        &case.headers,
        &case.litellm_params,
        case.api_key.as_deref(),
    );
    let merged = validate_environment(
        &setup.headers,
        setup.key.as_ref(),
        setup.policy.as_ref(),
        &setup.params,
    )
    .map_err(|error| status_failure(&error))?;
    Ok(json!({"headers": merged.into_iter().collect::<Vec<_>>()}))
}

fn error_view(error: &BedrockError) -> Value {
    let request = &error.error.request;
    let response = &error.error.response;
    json!({
        "status_code": error.error.status_code,
        "status_code_is_synthesized": error.error.status_code_is_synthesized,
        "message": error.error.message,
        "headers": error.response_headers,
        "request": {"method": request.method, "url": request.url},
        "response": {
            "status_code": response.status_code,
            "text": error_response_text(&error.response_body),
        },
    })
}

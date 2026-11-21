use std::collections::BTreeMap;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::responses_lifecycle_write::call::{
    LifecycleOperation, LifecycleWriteParams, ResponsesLifecycleWriteCall,
};
use betterlite_core::api::responses_lifecycle_write::error::ResponsesLifecycleWriteError;
use betterlite_core::api::responses_lifecycle_write::service::ResponsesLifecycleWrite;
use betterlite_core::api::responses_write_http::config::OpenAiResponsesConfig;
use betterlite_core::api::responses_write_http::service::WriteDeps;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::response_resources::response::TypedReply;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::result::{Failure, Nullable};
use super::support::{Fixture, Source, source};
use super::{Call, Mode, Outcome, Run, RunOutcome};

// The production service over the mock's base: the same config sections and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = loaded_config(env)?;
    let provider = OpenAiResponsesConfig::new(
        &config.sections.openai_credentials,
        LiteLlmGlobals::default(),
        config.sections.resource_sdk_settings.openai_api_key.clone(),
        None,
    );
    let origin = source(&config, env)?;
    let timeout =
        RequestTimeout::resolve(env).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let service = ResponsesLifecycleWrite {
        deps: WriteDeps {
            config: &provider,
            source: &origin,
            supplied: None,
            shared_session: None,
        },
        request_timeout: timeout,
    };
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(&service, run, api_base).await?);
    }
    Ok(SdkResult::ResponsesLifecycleWrite {
        value: Box::new(Outcome { runs }),
    })
}

async fn run_once(
    service: &ResponsesLifecycleWrite<'_, OpenAiResponsesConfig<'_>, Source>,
    run: &Run,
    api_base: &str,
) -> Result<RunOutcome, SdkHttpError> {
    let fixture = Fixture::new()?;
    let mut state = fixture.state()?;
    let env = fixture.pre_post();
    let redactor = &fixture.redactor;
    let request = request_of(run, api_base);
    match (run.operation, run.mode) {
        (LifecycleOperation::Delete, Mode::Sync) => outcome(
            service.delete_responses(&request, &mut state, &env).await,
            redactor,
            deleted_value,
        ),
        (LifecycleOperation::Delete, Mode::Asynchronous) => outcome(
            service.adelete_responses(&request, &mut state, &env).await,
            redactor,
            deleted_value,
        ),
        (LifecycleOperation::Cancel, Mode::Sync) => outcome(
            service.cancel_responses(&request, &mut state, &env).await,
            redactor,
            cancelled_value,
        ),
        (LifecycleOperation::Cancel, Mode::Asynchronous) => outcome(
            service.acancel_responses(&request, &mut state, &env).await,
            redactor,
            cancelled_value,
        ),
    }
}

fn loaded_config(env: &BTreeMap<String, String>) -> Result<Config, SdkHttpError> {
    ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))
}

// The kwargs the four entry points read, with the harness's base as the one the caller set
fn request_of(run: &Run, api_base: &str) -> ResponsesLifecycleWriteCall {
    ResponsesLifecycleWriteCall {
        operation: run.operation,
        response_id: run.response_id.clone(),
        custom_llm_provider: run.custom_llm_provider.clone(),
        extra_headers: run.extra_headers.clone(),
        extra_query: run.extra_query.clone(),
        extra_body: run.extra_body.clone(),
        timeout: run.timeout,
        params: LifecycleWriteParams {
            api_base: Some(api_base.to_owned()),
            api_key: run.params.api_key.clone().map(SecretString::from),
            ssl_verify: None,
            litellm_call_id: run.params.litellm_call_id.clone(),
            metadata: run.params.metadata.clone(),
            litellm_metadata: run.params.litellm_metadata.clone(),
        },
        adelete_responses: None,
        acancel_responses: None,
    }
}

// One run's answer as the case compares it: the projection of a success, or the SDK exception the entry raised
fn outcome<T>(
    answer: Result<T, ResponsesLifecycleWriteError>,
    redactor: &Redactor,
    project: fn(&T) -> Json,
) -> Result<RunOutcome, SdkHttpError> {
    match answer {
        Ok(value) => Ok(RunOutcome {
            result: Some(project(&value)),
            error: None,
        }),
        Err(error) => Ok(RunOutcome {
            result: None,
            error: Some(failure(&error, redactor)?),
        }),
    }
}

// The SDK exception the entry point raises, as the pinned SDK's caller can observe it
fn failure(
    error: &ResponsesLifecycleWriteError,
    redactor: &Redactor,
) -> Result<Failure, SdkHttpError> {
    let class = error.error_class(redactor);
    let metadata = class.metadata();
    Ok(Failure {
        class: class_name(&class)?.to_owned(),
        status: metadata.status_code,
        kind: field(metadata.error_type),
        param: field(metadata.param),
        code: Some(metadata.openai_code),
        message: error.to_string(),
    })
}

fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// only the classes the pinned SDK raises as openai.APIError subclasses have a projection here
fn class_name(class: &ErrorClass) -> Result<&'static str, SdkHttpError> {
    match class {
        ErrorClass::Authentication(_) => Ok("AuthenticationError"),
        ErrorClass::NotFound(_) => Ok("NotFoundError"),
        ErrorClass::PermissionDenied(_) => Ok("PermissionDeniedError"),
        ErrorClass::BadRequest(_) => Ok("BadRequestError"),
        ErrorClass::UnprocessableEntity(_) => Ok("UnprocessableEntityError"),
        ErrorClass::ContextWindowExceeded(_) => Ok("ContextWindowExceededError"),
        ErrorClass::ContentPolicyViolation(_) => Ok("ContentPolicyViolationError"),
        ErrorClass::ServiceUnavailable(_) => Ok("ServiceUnavailableError"),
        ErrorClass::BadGateway(_) => Ok("BadGatewayError"),
        ErrorClass::InternalServer(_) => Ok("InternalServerError"),
        ErrorClass::Timeout(_) => Ok("Timeout"),
        ErrorClass::RateLimit(_) => Ok("RateLimitError"),
        ErrorClass::Api(_) => Ok("APIError"),
        ErrorClass::ApiConnection(_) => Ok("APIConnectionError"),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}

fn deleted_value(record: &DeleteResponseResult) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        String::from("id"),
        record.id.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("object"),
        record.object.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("deleted"),
        record.deleted.map_or(Json::Null, Json::Bool),
    );
    Json::Object(fields)
}

fn cancelled_value(reply: &TypedReply) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("id"), Json::Text(reply.response.id.clone()));
    fields.insert(
        String::from("created_at"),
        Json::Number(Number::from(reply.response.created_at)),
    );
    fields.insert(
        String::from("object"),
        reply.response.object.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("status"),
        reply.response.status.clone().map_or(Json::Null, Json::Text),
    );
    fields.insert(
        String::from("model"),
        reply.response.model.clone().map_or(Json::Null, Json::Text),
    );
    Json::Object(fields)
}

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::api::responses_session_history::SessionLookupSettings;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chat_response_stream::driver::ChatResponseStream;
use betterlite_core::api::chat_response_stream_items::sources::EventSources;
use betterlite_core::api::responses_chat_call::error::{AsyncCallError, CallError};
use betterlite_core::api::responses_chat_call::handler::{
    async_response_handler, response_handler,
};
use betterlite_core::api::responses_chat_call::records::{
    CallOutcome, ResponseCall, ResponseChatCall,
};
use betterlite_core::api::responses_chat_call::source::ResponsesChunkSource;
use betterlite_core::api::responses_session_history::SessionHistory;
use betterlite_core::api::responses_tool_repair::value::Value as RepairValue;
use betterlite_core::shadow_requests::surfaces::environment::{
    NoBridge, NoParameters, empty_cache,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use crate::sdk_http::OracleLimits;
use crate::sdk_http::call_checks::validate_timeout;
use crate::sdk_http::call_kind::CallKind;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::kinds::SdkResult;
use crate::sdk_http::wrapper_stream::NORMALIZED_ID;

pub type Call = call::Call;
pub type Outcome = call::Outcome;

use assembly::StreamContext;
use call::{Mode, StreamView};
use client::{CompletionsError, HttpCompletions};
use session::{NoCold, NoSleep, RecordedQuery};

mod assembly;
mod call;
mod client;
mod session;

const PROVIDER: &str = "openai";

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        validate_timeout(self.timeout_seconds, limits)
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        async move {
            let outcome = run(self, api_base, env).await?;
            Ok(SdkResult::ResponsesChatCall {
                value: Box::new(outcome),
            })
        }
    }
}

async fn run(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Outcome, SdkHttpError> {
    let request = request_params(call)?;
    let completions = HttpCompletions::new(call, api_base, env)?;
    let parameters = NoParameters;
    let probe = NoBridge;
    let query = RecordedQuery::new(&call.history)?;
    let cache = empty_cache().map_err(SdkHttpError::Redactor)?;
    let sleeper = NoSleep;
    let tool_calls: IndexMap<String, RepairValue> = IndexMap::new();
    let history = session_history(&query, &sleeper, &cache, &tool_calls);
    let deps = call_deps(&completions, &cache, &parameters, &probe);
    let record = response_call(call, &request)?;
    let observed = match call.mode {
        Mode::Sync => match response_handler(&deps, record).await {
            Ok(outcome) => outcome,
            Err(error) => return failed_call(error, &completions, call, query.lookups()),
        },
        Mode::Async => match async_response_handler(&deps, &history, record).await {
            Ok(outcome) => outcome,
            Err(error) => return failed_async_call(error, &completions, call, query.lookups()),
        },
    };
    match observed {
        CallOutcome::Response(assembled) => Ok(Outcome {
            response: Some(as_json(&assembled.response)?),
            stream: None,
            failure: None,
            lookups: query.lookups(),
        }),
        CallOutcome::Stream(stream) => stream_outcome(stream, query.lookups()).await,
    }
}

fn request_params(call: &Call) -> Result<ResponsesApiRequestParams, SdkHttpError> {
    let encoded = serde_json::to_value(&call.request).map_err(SdkHttpError::Serialize)?;
    serde_json::from_value(encoded).map_err(|source| {
        SdkHttpError::Invalid(format!(
            "the case request is not a Responses request: {source}"
        ))
    })
}

fn session_history<'deps>(
    query: &'deps RecordedQuery,
    sleeper: &'deps NoSleep,
    cache: &'deps DualLocalCache,
    tool_calls: &'deps IndexMap<String, RepairValue>,
) -> SessionHistory<'deps, RecordedQuery, NoCold, NoSleep> {
    SessionHistory {
        repository: Some(query),
        cold_storage: None,
        sleeper,
        lookup: SessionLookupSettings {
            max_attempts: 1_u64,
            retry_interval_seconds: 0.2_f64,
        },
        disable_spend_logs: true,
        limits: ApiLimits::default(),
        input_cache: cache,
        tool_calls,
    }
}

fn call_deps<'deps>(
    completions: &'deps HttpCompletions,
    cache: &'deps DualLocalCache,
    parameters: &'deps NoParameters,
    probe: &'deps NoBridge,
) -> ResponseChatCall<'deps, HttpCompletions> {
    ResponseChatCall {
        completions,
        limits: ApiLimits::default(),
        cache,
        parameters,
        probe,
        route_all_openai_to_responses: false,
    }
}

fn response_call<'call>(
    call: &'call Call,
    request: &'call ResponsesApiRequestParams,
) -> Result<ResponseCall<'call, StreamContext>, SdkHttpError> {
    Ok(ResponseCall {
        model: &call.model,
        request,
        custom_llm_provider: Some(PROVIDER),
        stream: stream_flag(&call.request),
        extra_headers: call.extra_headers.as_ref(),
        kwargs: &call.kwargs,
        logging: None,
        sources: sources(),
        context: StreamContext::new()?,
    })
}

// The provider's answer is projected by B011, never turned into a Responses object
fn failed_call(
    error: CallError<CompletionsError>,
    completions: &HttpCompletions,
    call: &Call,
    lookups: Vec<String>,
) -> Result<Outcome, SdkHttpError> {
    let failure = match error {
        CallError::Completion(CompletionsError::Provider) => {
            completions.project_failure(&call.model)?
        }
        CallError::Completion(other) => {
            return Err(SdkHttpError::Invalid(format!(
                "the completion call failed: {other}"
            )));
        }
        CallError::Preparation(error) => {
            return Err(SdkHttpError::Invalid(format!(
                "the case request was refused: {error}"
            )));
        }
        CallError::Conversion(error) => {
            return Err(SdkHttpError::Invalid(format!(
                "the chat answer did not convert: {error}"
            )));
        }
    };
    Ok(Outcome {
        response: None,
        stream: None,
        failure: Some(failure),
        lookups,
    })
}

fn failed_async_call(
    error: AsyncCallError<CompletionsError, Infallible>,
    completions: &HttpCompletions,
    call: &Call,
    lookups: Vec<String>,
) -> Result<Outcome, SdkHttpError> {
    match error {
        AsyncCallError::Call(error) => failed_call(error, completions, call, lookups),
        AsyncCallError::Session(error) => Err(SdkHttpError::Invalid(format!(
            "the session lookup failed: {error}"
        ))),
    }
}

async fn stream_outcome(
    mut stream: Box<ChatResponseStream<ResponsesChunkSource, StreamContext>>,
    lookups: Vec<String>,
) -> Result<Outcome, SdkHttpError> {
    let view = StreamView {
        model: stream.state.model.clone(),
        custom_llm_provider: stream.state.custom_llm_provider.clone(),
        request_input: stream.state.request_input.clone(),
        responses_api_request: stream.state.responses_api_request.clone(),
        litellm_metadata: Json::Object(stream.state.litellm_metadata.clone()),
        event_types: Vec::new(),
    };
    let mut event_types = Vec::new();
    while let Some(event) = stream
        .next_event_async()
        .await
        .map_err(|error| SdkHttpError::Invalid(format!("the Responses stream failed: {error}")))?
    {
        event_types.push(event_name(&event)?);
    }
    Ok(Outcome {
        response: None,
        stream: Some(StreamView {
            event_types,
            ..view
        }),
        failure: None,
        lookups,
    })
}

fn event_name(event: &KnownResponseEvent) -> Result<String, SdkHttpError> {
    let value = serde_json::to_value(event).map_err(SdkHttpError::Serialize)?;
    let Value::Object(fields) = value else {
        return Err(SdkHttpError::Invalid(
            "a Responses event is not an object".to_owned(),
        ));
    };
    match fields.get("type") {
        Some(Value::String(name)) => Ok(name.clone()),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        )
        | None => Err(SdkHttpError::Invalid(
            "a Responses event carries no type".to_owned(),
        )),
    }
}

fn as_json<T>(value: &T) -> Result<Json, SdkHttpError>
where
    T: Serialize,
{
    let value = serde_json::to_value(value).map_err(SdkHttpError::Serialize)?;
    serde_json::from_value(value)
        .map_err(|source| SdkHttpError::Invalid(format!("a captured answer is not JSON: {source}")))
}

fn stream_flag(request: &Json) -> Option<bool> {
    let Json::Object(fields) = request else {
        return None;
    };
    match fields.get("stream") {
        Some(Json::Bool(value)) => Some(*value),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// LiteLLM reads time.time() and uuid.uuid4(); the oracle pins both so the state can be compared exactly
fn sources() -> EventSources {
    EventSources {
        clock: Arc::new(|| Ok(0_i64)),
        ids: Arc::new(|| NORMALIZED_ID.to_owned()),
    }
}

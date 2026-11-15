use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_context::agentic_callback::AgenticCallback;
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_core::api::responses_agentic_execution::AgenticResult;
use betterlite_core::api::responses_agentic_execution::dispatch::call_agentic_completion_hooks;
use betterlite_core::api::responses_agentic_execution::error::AgenticExecutionError;
use betterlite_core::api::responses_agentic_execution::executor::CompletionCall;
use betterlite_core::api::responses_agentic_execution::http::{
    DEFAULT_DEADLINE, ResponsesHttpExecutor,
};
use betterlite_core::api::responses_hook_policy::safety::ApiSurface;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};
use serde_json::{Map, Value};
use url::Url;

use super::error::SdkHttpError;
use super::result::{AgenticContinuationRun, AgenticFailure, AgenticTrace, SdkResult};
use callbacks::ScriptedCallback;

mod callbacks;

pub(crate) const PROVIDER: &str = "openai";
pub(crate) const PROVIDER_KEY: &str = "custom_llm_provider";

const TRACED_KEYS: [&str; 2] = [PROVIDER_KEY, "max_agentic_loops"];
const TRACED_PREFIX: &str = "_agentic_loop";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgenticContinuationCall {
    pub(crate) model: String,
    pub(crate) input: Json,
    #[serde(default)]
    pub(crate) params: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) kwargs: IndexMap<String, Json>,
    #[serde(default)]
    pub(crate) callbacks: Vec<CallbackScript>,
    #[serde(default)]
    pub(crate) dynamic: Vec<CallbackScript>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallbackScript {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) skip: bool,
    #[serde(default)]
    pub(crate) gate: Option<GateScript>,
    #[serde(default)]
    pub(crate) gate_error: Option<String>,
    #[serde(default)]
    pub(crate) plan: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) plan_error: Option<String>,
    #[serde(default)]
    pub(crate) legacy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateScript {
    pub(crate) run: bool,
    #[serde(default)]
    pub(crate) tools: IndexMap<String, Json>,
}

// the dispatcher's surface kwargs carry the provider the case claims to call
pub(crate) fn validate(call: &AgenticContinuationCall) -> Result<(), SdkHttpError> {
    let provider = call.kwargs.get(PROVIDER_KEY);
    if !matches!(provider, Some(Json::Text(name)) if name == PROVIDER) {
        return Err(SdkHttpError::Invalid(
            "the agentic continuation needs custom_llm_provider=openai".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) async fn send(
    call: &AgenticContinuationCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let client = super::vector_query_embedding::outbound_client(env)?;
    let response = initial_response(&client, api_base, call).await?;
    let executor = ResponsesHttpExecutor {
        client,
        api_base: api_base.to_owned(),
        provider: ProviderId::Openai,
        headers: HeaderMap::new(),
        model_info: None,
        deadline: DEFAULT_DEADLINE,
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let callbacks = scripts(call, &trace);
    let messages: Vec<FollowupMessage> = Vec::new();
    let optional_params = raw_params(&call.params)?;
    let completion = CompletionCall {
        response: &response,
        model: &call.model,
        messages: &messages,
        optional_params: &optional_params,
        logging: None,
        stream: false,
        custom_llm_provider: PROVIDER,
        kwargs: &call.kwargs,
        api_surface: ApiSurface::Responses,
    };
    let value = match call_agentic_completion_hooks(&completion, &callbacks, &executor).await {
        Ok(Some(AgenticResult::Response(owned))) => AgenticContinuationRun {
            trace: collected(&trace),
            returned: Some(serde_json::from_str(owned.get()).map_err(SdkHttpError::ResultParse)?),
            error: None,
        },
        Ok(Some(AgenticResult::Stream(_))) => {
            return Err(SdkHttpError::Invalid(
                "the responses surface wrapped its answer in a fake stream".to_owned(),
            ));
        }
        Ok(None) => AgenticContinuationRun {
            trace: collected(&trace),
            returned: Some(
                serde_json::from_str(response.get()).map_err(SdkHttpError::ResultParse)?,
            ),
            error: None,
        },
        Err(AgenticExecutionError::Safety(error)) => AgenticContinuationRun {
            trace: collected(&trace),
            returned: None,
            error: Some(AgenticFailure {
                class: "APIConnectionError".to_owned(),
                message: error.to_string(),
            }),
        },
        Err(error) => {
            return Err(SdkHttpError::Invalid(format!(
                "the native agentic dispatcher failed: {error}"
            )));
        }
    };
    Ok(SdkResult::ResponsesAgenticContinuation { value })
}

// the mock stands in for the provider, so the harness's base replaces the caller's
async fn initial_response(
    client: &ProxiedClient,
    api_base: &str,
    call: &AgenticContinuationCall,
) -> Result<Box<RawValue>, SdkHttpError> {
    let url = format!("{}/responses", api_base.trim_end_matches('/'));
    let request = OutboundRequest {
        method: Method::POST,
        url: Url::parse(&url).map_err(|source| SdkHttpError::Url(Box::new(source)))?,
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(initial_body(call)?),
        deadline: DEFAULT_DEADLINE,
    };
    let response = client
        .send_buffered(request)
        .await
        .map_err(|source| SdkHttpError::Transport(Box::new(source)))?;
    if !(200..300).contains(&response.status) {
        return Err(SdkHttpError::Invalid(format!(
            "the mock answered the responses call with status {}",
            response.status
        )));
    }
    let text = String::from_utf8(response.body).map_err(SdkHttpError::ErrorBodyUtf8)?;
    RawValue::from_string(text).map_err(SdkHttpError::ResultParse)
}

fn initial_body(call: &AgenticContinuationCall) -> Result<Vec<u8>, SdkHttpError> {
    let mut body = Map::new();
    for (key, value) in &call.params {
        body.insert(
            key.clone(),
            serde_json::to_value(value).map_err(SdkHttpError::Serialize)?,
        );
    }
    body.insert("model".to_owned(), Value::String(call.model.clone()));
    body.insert(
        "input".to_owned(),
        serde_json::to_value(&call.input).map_err(SdkHttpError::Serialize)?,
    );
    serde_json::to_vec(&Value::Object(body)).map_err(SdkHttpError::Serialize)
}

fn raw_params(params: &IndexMap<String, Json>) -> Result<JsonMap, SdkHttpError> {
    let mut map = JsonMap::with_capacity(params.len());
    for (key, value) in params {
        map.insert(
            key.clone(),
            to_raw_value(value).map_err(SdkHttpError::Serialize)?,
        );
    }
    Ok(map)
}

// a string callback is skipped by the dispatcher, so only the Python list carries it
fn scripts(
    call: &AgenticContinuationCall,
    trace: &Arc<Mutex<Vec<AgenticTrace>>>,
) -> Vec<Arc<dyn AgenticCallback>> {
    let mut callbacks: Vec<Arc<dyn AgenticCallback>> = Vec::new();
    for script in call.callbacks.iter().chain(&call.dynamic) {
        if script.skip {
            continue;
        }
        callbacks.push(Arc::new(ScriptedCallback {
            script: script.clone(),
            trace: Arc::clone(trace),
        }));
    }
    callbacks
}

fn collected(trace: &Arc<Mutex<Vec<AgenticTrace>>>) -> Vec<AgenticTrace> {
    match trace.lock() {
        Ok(trace) => trace.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

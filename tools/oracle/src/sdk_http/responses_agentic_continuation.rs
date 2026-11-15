use super::error::SdkHttpError;
use super::result::{AgenticContinuationRun, AgenticFailure, AgenticIdentity, SdkResult};
use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, FollowupMessage, JsonMap};
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
use std::collections::BTreeMap;
use std::sync::Arc;
use url::Url;
pub(crate) const PROVIDER: &str = "openai";
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
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GateScript {
    pub(crate) run: bool,
    #[serde(default)]
    pub(crate) tools: IndexMap<String, Json>,
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
    let outcome = call_agentic_completion_hooks(&completion, &scripts(call), &executor).await;
    let value = continuation_run(outcome, &response)?;
    Ok(SdkResult::ResponsesAgenticContinuation { value })
}
fn continuation_run(
    outcome: Result<Option<AgenticResult>, AgenticExecutionError>,
    initial: &RawValue,
) -> Result<AgenticContinuationRun, SdkHttpError> {
    match outcome {
        Ok(None) => answered(initial),
        Ok(Some(AgenticResult::Response(response))) => answered(&response),
        Ok(Some(AgenticResult::Stream(_))) => Err(SdkHttpError::Invalid(
            "the responses surface wrapped its answer in a fake stream".to_owned(),
        )),
        Err(AgenticExecutionError::Safety(error)) => Ok(AgenticContinuationRun {
            returned: None,
            error: Some(AgenticFailure {
                class: "APIConnectionError".to_owned(),
                message: error.to_string(),
            }),
        }),
        Err(error) => Err(SdkHttpError::Invalid(format!(
            "the native agentic dispatcher failed: {error}"
        ))),
    }
}
fn answered(identity: &RawValue) -> Result<AgenticContinuationRun, SdkHttpError> {
    let returned: AgenticIdentity =
        serde_json::from_str(identity.get()).map_err(SdkHttpError::ResultParse)?;
    Ok(AgenticContinuationRun {
        returned: Some(returned),
        error: None,
    })
}
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
        let encoded = serde_json::to_value(value).map_err(SdkHttpError::Serialize)?;
        body.insert(key.clone(), encoded);
    }
    body.insert("model".to_owned(), Value::String(call.model.clone()));
    let input = serde_json::to_value(&call.input).map_err(SdkHttpError::Serialize)?;
    body.insert("input".to_owned(), input);
    serde_json::to_vec(&Value::Object(body)).map_err(SdkHttpError::Serialize)
}
fn raw_params(params: &IndexMap<String, Json>) -> Result<JsonMap, SdkHttpError> {
    let mut map = JsonMap::with_capacity(params.len());
    for (key, value) in params {
        let raw = to_raw_value(value).map_err(SdkHttpError::Serialize)?;
        map.insert(key.clone(), raw);
    }
    Ok(map)
}
fn scripts(call: &AgenticContinuationCall) -> Vec<Arc<dyn AgenticCallback>> {
    let mut callbacks: Vec<Arc<dyn AgenticCallback>> = Vec::new();
    for script in call.callbacks.iter().chain(&call.dynamic) {
        if !script.skip {
            callbacks.push(Arc::new(ScriptedCallback {
                script: script.clone(),
            }));
        }
    }
    callbacks
}
struct ScriptedCallback {
    script: CallbackScript,
}
impl ScriptedCallback {
    fn gate_tools(&self, tools: &IndexMap<String, Json>) -> Result<JsonMap, AgenticHookError> {
        let mut map = JsonMap::with_capacity(tools.len());
        for (key, value) in tools {
            let raw = to_raw_value(value).map_err(|source| AgenticHookError::Failed {
                callback: self.script.name.clone(),
                message: format!("the case gate tools do not encode: {source}"),
            })?;
            map.insert(key.clone(), raw);
        }
        Ok(map)
    }
    fn case_plan(
        &self,
        plan: &IndexMap<String, Json>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        let text = serde_json::to_string(plan).map_err(|source| AgenticHookError::Failed {
            callback: self.script.name.clone(),
            message: format!("the case plan does not encode: {source}"),
        })?;
        serde_json::from_str::<AgenticLoopPlan>(&text).map_err(|source| AgenticHookError::Failed {
            callback: self.script.name.clone(),
            message: format!("the case plan does not decode: {source}"),
        })
    }
}
#[async_trait]
impl AgenticCallback for ScriptedCallback {
    fn callback_name(&self) -> &str {
        &self.script.name
    }
    fn overrides(&self) -> AgenticOverrides {
        let plans = self.script.plan.is_some() || self.script.plan_error.is_some();
        AgenticOverrides {
            should_run: true,
            run_loop: false,
            build_plan: plans,
            post_response: plans,
            cleanup: plans,
        }
    }
    async fn should_run_agentic_loop(
        &self,
        _call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        if let Some(message) = &self.script.gate_error {
            return Err(AgenticHookError::Failed {
                callback: self.script.name.clone(),
                message: message.clone(),
            });
        }
        let Some(gate) = &self.script.gate else {
            return Ok(AgenticGate::default());
        };
        Ok(AgenticGate {
            run: gate.run,
            tools: self.gate_tools(&gate.tools)?,
        })
    }
    async fn build_agentic_loop_plan(
        &self,
        _call: &AgenticLoopCall<'_>,
    ) -> Result<AgenticLoopPlan, AgenticHookError> {
        match (&self.script.plan_error, &self.script.plan) {
            (Some(message), _) => Err(AgenticHookError::Failed {
                callback: self.script.name.clone(),
                message: message.clone(),
            }),
            (None, Some(plan)) => self.case_plan(plan),
            (None, None) => Ok(AgenticLoopPlan::default()),
        }
    }
}

use crate::sdk_http::call_kind::CallKind as _;
use crate::sdk_http::responses_chat_call::call::{Call as ChatCall, Mode, Outcome as ChatOutcome};
use crate::sdk_http::result::kinds::SdkResult;
use async_trait::async_trait;
use betterlite_core::api::responses_agentic_execution::executor::{
    AnthropicContinuation, ContinuationExecutor, ResponsesContinuation,
};
use betterlite_core::api::responses_chat_call::records::CallOutcome;
use betterlite_core::api::responses_chat_output::records::AssembledResponse;
use betterlite_core::api::responses_create::bridge::{BridgeCall, ChatBridge, Created};
use betterlite_core::api::responses_create::convert::{decision_json, reencode};
use betterlite_core::api::responses_create::error::{BridgeError, InterceptionError};
use betterlite_core::api::responses_create::interception::{
    CreateInterception, DispatchArgs, FileSearchExtras, Interception,
};
use betterlite_core::api::responses_create_http::config::Params;
use betterlite_core::api::responses_hook_policy::callbacks::{
    DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
};
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::time::Duration;
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct HookFailure {
    pub(super) message: String,
}
fn hook_failure(message: &str) -> HookFailure {
    HookFailure {
        message: message.to_owned(),
    }
}
// No deployment hooks, callbacks or continuations are installed before the outer composition
pub(super) struct Nothing;
#[async_trait]
impl DeploymentHookDispatcher for Nothing {
    type Error = HookFailure;
    async fn pre_call_deployment_hook(
        &self,
        _request: Params,
        _call_type: &str,
    ) -> Result<Option<Params>, Self::Error> {
        Ok(None)
    }
}
impl NamedCallbackResolver for Nothing {
    fn resolve(&self, _name: &str) -> Option<PolicyCallback> {
        None
    }
}
#[async_trait]
impl ContinuationExecutor for Nothing {
    type Error = HookFailure;
    async fn responses(&self, _call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        Err(hook_failure("a continuation is not expected"))
    }
    async fn anthropic_messages(
        &self,
        _call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(hook_failure("a continuation is not expected"))
    }
}
// Neither dispatch is installed before the outer composition, so both answers are Continue
pub(super) struct NoInterception;
#[async_trait]
impl CreateInterception<()> for NoInterception {
    fn mcp_requested(&self, _tools: Option<&[JsonValue]>) -> bool {
        false
    }
    async fn mcp(
        &self,
        _args: &DispatchArgs<'_>,
        context: (),
    ) -> Result<Interception<()>, InterceptionError> {
        Ok(Interception::Continue(context))
    }
    async fn file_search(
        &self,
        _args: &DispatchArgs<'_>,
        _extras: &FileSearchExtras<'_>,
        context: (),
    ) -> Result<Interception<()>, InterceptionError> {
        Ok(Interception::Continue(context))
    }
}
// The chat bridge B916's handler answers, run from the same case env and mock as the create call itself
pub(super) struct Bridge {
    pub(super) api_base: String,
    pub(super) env: BTreeMap<String, String>,
}
#[async_trait]
impl ChatBridge<()> for Bridge {
    async fn respond(
        &self,
        call: &BridgeCall<'_>,
        _context: (),
    ) -> Result<Created<()>, BridgeError> {
        let nested = ChatCall {
            mode: Mode::Sync,
            model: call.model.to_owned(),
            request: chat_request(call)?,
            api_key: self.env.get("OPENAI_API_KEY").cloned().unwrap_or_default(),
            timeout_seconds: timeout_seconds(call.kwargs),
            extra_headers: call.extra_headers.map(|headers| {
                Json::Object(
                    headers
                        .iter()
                        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
                        .collect(),
                )
            }),
            kwargs: call
                .kwargs
                .iter()
                .map(|(name, value)| (name.clone(), decision_json(value)))
                .collect(),
            history: Vec::new(),
        };
        match nested.send(&self.api_base, &self.env).await {
            Ok(SdkResult::ResponsesChatCall { value }) => chat_created(*value),
            Ok(other) => Err(bridge_failure(&format!("the chat call answered {other:?}"))),
            Err(error) => Err(bridge_failure(&error.to_string())),
        }
    }
}
fn bridge_failure(message: &str) -> BridgeError {
    BridgeError {
        message: message.to_owned(),
    }
}
// response_api_handler's model, input and Responses request, which B916 converts into the chat call
fn chat_request(call: &BridgeCall<'_>) -> Result<Json, BridgeError> {
    let mut request: Json =
        reencode(call.request).map_err(|error| bridge_failure(&error.to_string()))?;
    let Json::Object(fields) = &mut request else {
        return Err(bridge_failure("the bridge request is not an object"));
    };
    fields.insert(String::from("model"), Json::Text(call.model.to_owned()));
    fields.insert(
        String::from("input"),
        reencode(call.input).map_err(|error| bridge_failure(&error.to_string()))?,
    );
    Ok(request)
}
fn timeout_seconds(kwargs: &ParameterMap) -> u64 {
    let Some(JsonValue::Number(number)) = kwargs.get("timeout") else {
        return 0;
    };
    number
        .as_f64()
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .map_or(0, |duration| duration.as_secs())
}
fn chat_created(outcome: ChatOutcome) -> Result<Created<()>, BridgeError> {
    let Some(response) = outcome.response else {
        return Err(bridge_failure(&outcome.failure.map_or_else(
            || String::from("the chat call returned no response"),
            |failure| failure.message,
        )));
    };
    let response: ResponsesApiResponse =
        reencode(&response).map_err(|error| bridge_failure(&error.to_string()))?;
    Ok(Created::Chat(Box::new(CallOutcome::Response(Box::new(
        AssembledResponse {
            response,
            hidden_params: BTreeMap::new(),
            cache_writes: Vec::new(),
        },
    )))))
}

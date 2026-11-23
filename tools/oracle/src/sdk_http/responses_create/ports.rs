use crate::sdk_http::call_kind::CallKind as _;
use crate::sdk_http::responses_chat_call::call::{Call as ChatCall, Mode, Outcome as ChatOutcome};
use crate::sdk_http::result::kinds::SdkResult;
use async_trait::async_trait;
use betterlite_core::api::responses_create::bridge::{BridgeCall, ChatBridge, Created};
use betterlite_core::api::responses_create::convert::{decision_json, reencode};
use betterlite_core::api::responses_create::error::BridgeError;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use serde_json::value::to_raw_value;
use std::collections::BTreeMap;
use std::time::Duration;
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
    let seconds = number.as_f64().unwrap_or(0.0_f64);
    let duration = Duration::try_from_secs_f64(seconds).unwrap_or_default();
    duration.as_secs()
}
fn chat_created(outcome: ChatOutcome) -> Result<Created<()>, BridgeError> {
    let Some(response) = outcome.response else {
        return Err(bridge_failure(&outcome.failure.map_or_else(
            || String::from("the chat call returned no response"),
            |failure| failure.message,
        )));
    };
    to_raw_value(&response)
        .map(Created::Raw)
        .map_err(|error| bridge_failure(&error.to_string()))
}

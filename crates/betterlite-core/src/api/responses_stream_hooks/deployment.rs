use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use indexmap::IndexMap;
use serde::Deserialize as _;
use serde::de::value::{Error as ValueError, StrDeserializer};

use super::ResponsesStreamHooks;
use super::ports::ChunkHookCall;
use super::state::ResponsesStreamHookState;
use crate::routes::CallType;

pub struct ChunkHookContext<'call> {
    pub explicit_call_type: Option<&'call str>,
    pub logging_call_type: Option<&'call str>,
    pub request_data: Option<&'call IndexMap<String, Json>>,
    pub model_call_details: &'call IndexMap<String, Json>,
}

#[must_use]
pub fn resolve_call_type(explicit: Option<&str>, logging: Option<&str>) -> Option<CallType> {
    explicit
        .and_then(valid_call_type)
        .or_else(|| logging.and_then(valid_call_type))
}

#[must_use]
pub fn call_type_name(call_type: CallType) -> Option<String> {
    match serde_json::to_value(call_type) {
        Ok(value) => value.as_str().map(str::to_owned),
        Err(_) => None,
    }
}

// an unknown name reaches the hooks as None, exactly as CallTypes(value) raising ValueError does
fn valid_call_type(name: &str) -> Option<CallType> {
    let parsed: Result<CallType, ValueError> =
        CallType::deserialize(StrDeserializer::<ValueError>::new(name));
    parsed.ok()
}

impl ResponsesStreamHooks<'_> {
    pub async fn call_post_streaming_deployment_hook(
        &self,
        callback: &mut ResponsesStreamHookState,
        context: ChunkHookContext<'_>,
        mut chunk: ResponsesAPIStreamingResponse,
    ) -> ResponsesAPIStreamingResponse {
        let call_type = resolve_call_type(context.explicit_call_type, context.logging_call_type);
        let request_data = match context.request_data {
            Some(data) if !data.is_empty() => data,
            Some(_) | None => context.model_call_details,
        };
        let mut hooks_ran = false;
        for hook in &self.chunk_hooks {
            hooks_ran = true;
            let call = ChunkHookCall {
                request_data,
                chunk: &chunk,
                call_type,
            };
            match hook.post_streaming_deployment_hook(call).await {
                Ok(Some(replacement)) => chunk = replacement,
                Ok(None) => {}
                Err(_) => return chunk,
            }
        }
        if hooks_ran {
            callback.post_streaming_hooks_ran = true;
        }
        chunk
    }
}

use betterlite_models::decisions::Json;
use serde_json::value::RawValue;

use super::error::WrapError;
use super::{AgenticResult, LoggingView};
use crate::api::messages_fake_stream::MessagesFakeStream;
use crate::api::responses_hook_policy::safety::ApiSurface;

// _maybe_wrap_in_fake_stream: the flag is only ever set for anthropic websearch interception
pub fn maybe_wrap_in_fake_stream(
    response: Box<RawValue>,
    logging: Option<LoggingView<'_>>,
    api_surface: ApiSurface,
) -> Result<AgenticResult, WrapError> {
    if api_surface != ApiSurface::AnthropicMessages {
        return Ok(AgenticResult::Response(response));
    }
    if !logging.is_some_and(|view| view.websearch_converted_stream) {
        return Ok(AgenticResult::Response(response));
    }
    let value: Json = serde_json::from_str(response.get())?;
    if !matches!(value, Json::Object(_)) {
        return Ok(AgenticResult::Response(response));
    }
    tracing::debug!(
        "WebSearchInterception: Agentic loop completed, converting non-streaming response to fake stream"
    );
    Ok(AgenticResult::Stream(MessagesFakeStream::new(&value)?))
}

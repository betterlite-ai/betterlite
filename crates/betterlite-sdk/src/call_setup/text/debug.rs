use betterlite_models::decisions::Json;
use betterlite_redact::structured::redact::redact_credentials_in_payload;
use thiserror::Error;

use crate::call_setup::RequestMap;

#[derive(Debug, Error)]
pub enum DebugTextError {
    #[error("the request could not be encoded for the debug line: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("the encoded request is not an object")]
    Shape,
}

pub fn debug_text(
    original_function: &str,
    request: &RequestMap,
    debugging: bool,
) -> Result<Option<String>, DebugTextError> {
    if !debugging || inner_async_call(original_function, request) {
        return Ok(None);
    }
    let encoded = serde_json::to_value(Json::Object(request.clone()))?;
    let Some(fields) = encoded.as_object() else {
        return Err(DebugTextError::Shape);
    };
    let redacted = redact_credentials_in_payload(fields);
    let mut rendered = Vec::new();
    if let Some(redacted_fields) = redacted.as_object() {
        for (key, value) in redacted_fields {
            rendered.push(format!("{key}={value}"));
        }
    }
    let arguments = rendered.join(", ");
    let call = format!("litellm.{original_function}({arguments})");
    let text = format!("\n\x1b[92mRequest to litellm:\x1b[0m\n\x1b[92m{call}\x1b[0m\n");
    Ok(Some(text))
}

// LiteLLM already printed the pair for the async entry point, so the inner sync function stays quiet
fn inner_async_call(original_function: &str, request: &RequestMap) -> bool {
    let pairs = [
        ("acompletion", "completion"),
        ("aembedding", "embedding"),
        ("aimg_generation", "img_generation"),
    ];
    pairs.iter().any(|(flag, name)| {
        *name == original_function && matches!(request.get(*flag), Some(Json::Bool(true)))
    })
}

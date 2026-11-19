use betterlite_anthropic::anthropic_http_stream::error::StreamFailure;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::anthropic_dispatch::native::Counter;

pub(crate) mod call;
pub(crate) mod context;
pub(crate) mod error;
pub(crate) mod files;
pub(crate) mod frames;
pub(crate) mod logging;
pub(crate) mod parameters;
pub(crate) mod reply;
pub(crate) mod runtime;

pub use reply::AnthropicReply;

// What a Messages call hands back before decode: the provider headers LiteLLM projected and the raw outcome.
pub struct AnthropicUpstream {
    pub headers: HeaderMap,
    pub outcome: ChatOutcome<StreamFailure, Counter>,
}

pub(crate) fn provider_headers(outcome: &ChatOutcome<StreamFailure, Counter>) -> HeaderMap {
    match outcome {
        ChatOutcome::Reply(envelope) => additional_headers(&envelope.hidden_params),
        ChatOutcome::Stream(stream) => stream_headers(&stream.response_headers),
        ChatOutcome::Embedding => HeaderMap::new(),
    }
}

fn additional_headers(hidden: &IndexMap<String, Json>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let Some(Json::Object(additional)) = hidden.get("additional_headers") else {
        return headers;
    };
    for (name, value) in additional {
        if let Json::Text(text) = value {
            insert(&mut headers, name, text);
        }
    }
    headers
}

fn stream_headers(response: &IndexMap<String, String>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in response {
        insert(&mut headers, name, value);
    }
    headers
}

fn insert(headers: &mut HeaderMap, raw_name: &str, raw_value: &str) {
    let Ok(name) = HeaderName::try_from(raw_name) else {
        return;
    };
    let Ok(value) = HeaderValue::try_from(raw_value) else {
        return;
    };
    headers.insert(name, value);
}

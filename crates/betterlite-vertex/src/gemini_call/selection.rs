use std::time::Duration;

use betterlite_prompts::arguments::json::{Json, Object};

use crate::gemini_http::error::GeminiHttpError;
use crate::vertex_embedding_call::post::{ClientTimeout, Timeout};

// LiteLLM's two independent conditions: acompletion picks the async pair and only a literal true streams
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallSelection {
    pub asynchronous: bool,
    pub streaming: bool,
}

// LiteLLM's `if acompletion:` then `if stream is True:`
#[must_use]
pub const fn select(acompletion: bool, stream: Option<bool>) -> CallSelection {
    CallSelection {
        asynchronous: acompletion,
        streaming: matches!(stream, Some(true)),
    }
}

// LiteLLM pops stream before the transform arguments are built, so neither the cache nor the body sees it
pub fn pop_stream(optional_params: &mut Object) -> Result<Option<bool>, GeminiHttpError> {
    match optional_params.shift_remove("stream") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(stream)) => Ok(Some(stream)),
        Some(_) => Err(GeminiHttpError::Flag { field: "stream" }),
    }
}

// LiteLLM's `if timeout is not None:`: zero builds a client that fails at once, absent keeps the handler's default
#[must_use]
pub const fn sync_client_timeout(timeout: Option<Duration>) -> ClientTimeout {
    match timeout {
        Some(total) => ClientTimeout::Explicit(Timeout {
            total,
            connect: total,
        }),
        None => ClientTimeout::HandlerDefault,
    }
}

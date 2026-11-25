use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use serde::Deserialize;

use crate::api::messages_chat_output::output::MessageResponse;
use crate::api::messages_response_stream::MessagesResponseStream;
use crate::api::messages_responses_output::records::{AnthropicRequestRecord, SettingsRecord};
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_provider_selection::ParameterMap;

// litellm.max_tokens, which constants.DEFAULT_MAX_TOKENS gives an absent request max_tokens.
pub const DEFAULT_MAX_TOKENS: i64 = 4096;

// The settings this call reads: the adapter's own settings plus the proxy default the handler's required max_tokens needs.
#[derive(Debug, Clone, Deserialize)]
pub struct CallSettings {
    #[serde(default = "default_max_tokens")]
    pub default_max_tokens: i64,
    #[serde(flatten)]
    pub bridge: SettingsRecord,
}

impl Default for CallSettings {
    fn default() -> Self {
        Self {
            default_max_tokens: DEFAULT_MAX_TOKENS,
            bridge: SettingsRecord::default(),
        }
    }
}

const fn default_max_tokens() -> i64 {
    DEFAULT_MAX_TOKENS
}

// One call of LiteLLMMessagesToResponsesAPIHandler: the declared arguments, the litellm kwargs it forwards verbatim and the adapter settings it reads.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesResponseCallInput {
    pub request: AnthropicRequestRecord,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub settings: CallSettings,
    // the caller's **kwargs: provider credentials, deployment options and every provider-specific extra
    #[serde(default)]
    pub forwarded: ParameterMap,
}

// What litellm.responses/aresponses answered: the ResponsesAPIResponse the adapter translates, the stream it wraps, or a value the isinstance check refuses.
pub enum ResponsesAnswer<S> {
    Response(Box<ResponsesApiResponse>),
    Stream(S),
    Other { kind: String },
}

// The call that was sent and the Anthropic answer it produced; the call stays so the route logs what went upstream.
pub struct MessagesResponseCallResult<S> {
    pub call: ResponsesCreateCall,
    pub answer: MessagesResponseCallAnswer<S>,
}

pub enum MessagesResponseCallAnswer<S> {
    Native(Box<MessageResponse>),
    Stream(Box<MessagesResponseStream<S>>),
}

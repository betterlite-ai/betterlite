use betterlite_openai::chat::ChatMessage;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Supported {
        models: Vec<String>,
    },
    Vendor {
        models: Vec<String>,
    },
    Exchange {
        #[serde(default)]
        credentials: Object,
        exchanges: Vec<ExchangeCall>,
    },
    Response {
        responses: Vec<ResponseCall>,
    },
    Stream {
        reads: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExchangeCall {
    pub(super) model: String,
    pub(super) messages: Vec<ChatMessage>,
    pub(super) params: Object,
    pub(super) drop_params: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponseCall {
    pub(super) model: String,
    pub(super) status: u16,
    pub(super) body: Json,
}

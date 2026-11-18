use std::time::Duration;

use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::anthropic_messages::environment::{ParamsKey, TokenSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Async,
    Sync,
}

pub struct ChatRequest<'call> {
    pub model: &'call str,
    pub messages: Vec<Json>,
    pub optional_params: IndexMap<String, Json>,
    pub litellm_params: IndexMap<String, Json>,
    pub caller_headers: IndexMap<String, String>,
    pub api_base: &'call str,
    pub params_key: ParamsKey<'call>,
    pub argument_key: Option<&'call SecretString>,
    pub configured_key: Option<&'call SecretString>,
    pub token: TokenSource<'call>,
    pub timeout: Duration,
    pub style: Style,
    pub now: i64,
    pub disable_add_prefix_to_prompt: bool,
    pub model_response: ResponseEnvelope,
}

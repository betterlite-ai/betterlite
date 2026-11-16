use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use crate::snowflake::embedding::EmbeddingReply as SnowflakeEmbeddingReply;

#[derive(Debug, Clone)]
pub struct CompletionCall {
    pub model: String,
    pub stream: Option<bool>,
    pub acompletion: bool,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub custom_llm_provider: String,
    pub headers: Option<BTreeMap<String, String>>,
    pub litellm_params: Object,
    pub optional_params: Object,
    pub messages: Vec<Message>,
    pub timeout: Option<Duration>,
    pub provider_config: Option<ChatConfigTag>,
}

#[derive(Debug, Clone)]
pub struct EmbeddingCall {
    pub model: String,
    pub input: Json,
    pub optional_params: Object,
    pub litellm_params: Object,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub enum EmbeddingReply {
    Databricks(Box<EmbeddingResponse>),
    Snowflake(Box<SnowflakeEmbeddingReply>),
    Oci(Box<EmbeddingResponse>),
}

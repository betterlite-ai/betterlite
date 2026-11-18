use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Number, Value};

// the placeholder a case uses for the workspace it cannot know when it is written
pub(super) const PLACEHOLDER: &str = "{{upstream}}";
pub(super) const ORACLE_UPSTREAM: &str = "https://workspace.example.com";

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    SapChat,
    SapEmbedding,
    WatsonxChat,
    WatsonxText,
    WatsonxEmbedding,
    SagemakerChat,
    SagemakerNova,
}

impl Operation {
    // the headers LiteLLM's case scripts record for this provider
    pub(super) const fn headers(self) -> &'static [&'static str] {
        match self {
            Self::SapChat | Self::SapEmbedding => &[
                "authorization",
                "content-type",
                "ai-resource-group",
                "ai-client-type",
            ],
            Self::WatsonxChat | Self::WatsonxText | Self::WatsonxEmbedding => {
                &["authorization", "content-type", "accept"]
            }
            Self::SagemakerChat | Self::SagemakerNova => &[
                "authorization",
                "content-type",
                "x-amz-date",
                "x-amz-security-token",
            ],
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    pub(super) op: Operation,
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    input: Option<Input>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    pub(super) stream: Option<bool>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    max_tokens: Option<i64>,
    #[serde(default)]
    wx_credentials: Option<BTreeMap<String, String>>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) upstream: Vec<Reply>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Input {
    One(String),
    Many(Vec<String>),
}

impl Case {
    pub(super) fn environment(&self) -> BTreeMap<String, String> {
        self.env
            .iter()
            .map(|(name, value)| (name.clone(), value.replace(PLACEHOLDER, ORACLE_UPSTREAM)))
            .collect()
    }

    pub(super) fn api_base(&self) -> Option<String> {
        self.api_base
            .as_ref()
            .map(|base| base.replace(PLACEHOLDER, ORACLE_UPSTREAM))
    }

    pub(super) fn key(&self) -> Option<SecretString> {
        self.api_key
            .as_deref()
            .map(String::from)
            .map(SecretString::from)
    }

    // the embedding input as the caller wrote it: one text or a list of them
    pub(super) fn embedding_input(&self) -> Json {
        match &self.input {
            None => Json::Null,
            Some(Input::One(text)) => Json::Text(text.clone()),
            Some(Input::Many(items)) => Json::List(items.iter().cloned().map(Json::Text).collect()),
        }
    }

    // the caller's non-default params, which each watsonx mapper turns into its own optional params
    pub(super) fn non_default(&self) -> WatsonxParams {
        let mut params = WatsonxParams::new();
        if let Some(value) = self.temperature.and_then(Number::from_f64) {
            params.insert(String::from("temperature"), Value::Number(value));
        }
        if let Some(value) = self.max_tokens {
            params.insert(
                String::from("max_tokens"),
                Value::Number(Number::from(value)),
            );
        }
        params
    }

    // the optional params the watsonx text arm pops wx_credentials out of
    pub(super) fn credentials_params(&self) -> Object {
        let mut params = Object::new();
        if let Some(credentials) = &self.wx_credentials {
            let fields = credentials
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        Json::Text(value.replace(PLACEHOLDER, ORACLE_UPSTREAM)),
                    )
                })
                .collect();
            params.insert(String::from("wx_credentials"), Json::Object(fields));
        }
        params
    }
}

// the chat arm's non-default map for B5971's mapper
pub(super) fn chat_options(params: &WatsonxParams, stream: bool) -> JsonMap {
    let mut options = JsonMap::new();
    for (name, value) in params {
        options.insert(name.clone(), value.clone());
    }
    options.insert(String::from("stream"), Value::Bool(stream));
    options
}

// the body OpenAILike builds for watsonx chat: the model, the messages, then every optional param
pub(super) fn chat_body(model: String, messages: Value, options: WatsonxParams) -> Value {
    let mut body = Map::new();
    body.insert(String::from("model"), Value::String(model));
    body.insert(String::from("messages"), messages);
    for (name, value) in options {
        body.insert(name, value);
    }
    Value::Object(body)
}

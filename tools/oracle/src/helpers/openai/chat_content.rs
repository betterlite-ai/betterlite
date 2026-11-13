use std::collections::BTreeMap;

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ChatContentCall {
    TransformRequests(TransformRequestsInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformRequestsInput {
    requests: Vec<RequestInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    model: String,
    #[serde(default)]
    messages: Vec<ChatMessage>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    global_api_base: Option<String>,
    #[serde(default)]
    system_messages_first: bool,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    mode: Mode,
    #[serde(default)]
    downloads: BTreeMap<String, String>,
}

struct MapDownloads<'map>(&'map BTreeMap<String, String>);

impl UrlDownloader for MapDownloads<'_> {
    fn download(&self, url: &str) -> Result<String, ImageFetchError> {
        self.0
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Disabled {
                url: url.to_owned(),
            })
    }
}

fn request_body(input: &RequestInput) -> Result<Value, ChatContentError> {
    let content = ChatContent {
        model: &input.model,
        messages: &input.messages,
        optional_params: &input.optional_params,
        provider: input.provider.as_deref(),
        api_base: input.api_base.as_deref(),
        global_api_base: input.global_api_base.as_deref(),
        openai_system_messages_first: input.system_messages_first,
        environment: &input.environment,
        downloads: &MapDownloads(&input.downloads),
    };
    let body = content.transform(input.mode)?.to_body()?;
    serde_json::to_value(body).map_err(|source| ChatContentError::MessageShape { source })
}

fn outcome(input: &RequestInput) -> Value {
    match request_body(input) {
        Ok(body) => json!({"ok": true, "value": body}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = ChatContentCall;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            ChatContentCall::TransformRequests(value) => {
                Ok(Value::Array(value.requests.iter().map(outcome).collect()))
            }
        }
    }
}

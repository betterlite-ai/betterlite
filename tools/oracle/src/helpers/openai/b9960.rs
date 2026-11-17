use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{ChatContent, ChatContentError, Mode, RefuseDownloads};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ChatContentFetchCall {
    TransformRequests(TransformRequests),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformRequests {
    requests: Vec<Request>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
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
    urls: BTreeMap<String, String>,
    #[serde(default)]
    refuse: bool,
}

struct MapFetcher<'urls> {
    urls: &'urls BTreeMap<String, String>,
}

impl RemoteMediaFetcher for MapFetcher<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(mapped(self.urls, url))
    }

    // LiteLLM converts one URL per content item, so a batch call must fail the case instead of answering it
    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.first().map_or_else(
            || Ok(Vec::new()),
            |url| Err(ImageFetchError::Attempts { url: url.clone() }),
        ))
    }
}

fn mapped(urls: &BTreeMap<String, String>, url: &str) -> Result<String, ImageFetchError> {
    urls.get(url)
        .cloned()
        .ok_or_else(|| ImageFetchError::Attempts {
            url: url.to_owned(),
        })
}

fn body(input: &Request, runtime: &Runtime) -> Result<Value, ChatContentError> {
    let content = ChatContent {
        model: &input.model,
        messages: &input.messages,
        optional_params: &input.optional_params,
        provider: input.provider.as_deref(),
        api_base: input.api_base.as_deref(),
        global_api_base: input.global_api_base.as_deref(),
        openai_system_messages_first: input.system_messages_first,
        environment: &input.environment,
        downloads: &RefuseDownloads,
    };
    let request = if input.refuse {
        runtime.block_on(content.transform_with_media(input.mode, &RefuseDownloads))?
    } else {
        let fetcher = MapFetcher { urls: &input.urls };
        runtime.block_on(content.transform_with_media(input.mode, &fetcher))?
    };
    serde_json::to_value(request.to_body()?)
        .map_err(|source| ChatContentError::MessageShape { source })
}

fn outcome(input: &Request, runtime: &Runtime) -> Value {
    match body(input, runtime) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = ChatContentFetchCall;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            ChatContentFetchCall::TransformRequests(value) => {
                let runtime = Builder::new_current_thread().build().or_value_error()?;
                Ok(Value::Array(
                    value
                        .requests
                        .iter()
                        .map(|request| outcome(request, &runtime))
                        .collect(),
                ))
            }
        }
    }
}

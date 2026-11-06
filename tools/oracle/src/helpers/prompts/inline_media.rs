use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_prompts::media::base64_string::{get_base64_str, is_base64_encoded};
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::inline::{
    RemoteMediaFetcher, async_inline_remote_media, inline_remote_media, remote_urls_to_inline,
};
use betterlite_prompts::media::parts::{
    INLINE_EVERY_REMOTE_URL, INLINE_REMOTE_IMAGE_URLS, RemoteMedia,
};
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::media_fetch::fetcher::Fetcher;
use betterlite_prompts::media_fetch::limits::transport_limits;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Runtime;

use crate::helpers::{Helper, HelperError, HelperResult};

// LiteLLM's default MAX_IMAGE_URL_DOWNLOAD_SIZE_MB; the cases never set the variable
const LIMIT_MB: f64 = 50.0;

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Sequential,
    Concurrent,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Selection {
    #[default]
    Every,
    ImageUrls,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Inline {
        #[serde(default)]
        mode: Mode,
        #[serde(default)]
        should_inline: Selection,
        messages: Vec<Message>,
        data_urls: BTreeMap<String, String>,
    },
    Fetch {
        messages: Vec<Message>,
    },
    GetBase64Str {
        values: Vec<String>,
    },
    IsBase64Encoded {
        values: Vec<String>,
    },
}

struct MapSource<'data> {
    data_urls: &'data BTreeMap<String, String>,
}

impl MapSource<'_> {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls.get(url).map_or_else(
            || {
                Err(ImageFetchError::Attempts {
                    url: url.to_owned(),
                })
            },
            |data_url| Ok(data_url.clone()),
        )
    }
}

impl RemoteMediaFetcher for MapSource<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.result_for(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let results = urls.iter().map(|url| self.result_for(url)).collect();
        ready(results)
    }
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn media_error(error: &MediaError) -> Box<HelperError> {
    let (class, status) = match error {
        MediaError::Fetch(_) => ("ImageFetchError", Some(400)),
        MediaError::MissingDataPrefix
        | MediaError::MissingBase64Marker
        | MediaError::TooLarge { .. }
        | MediaError::InvalidBase64(_) => ("ValueError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid_input(error.to_string()))
}

fn predicate(selection: Selection) -> fn(&RemoteMedia) -> bool {
    match selection {
        Selection::Every => INLINE_EVERY_REMOTE_URL,
        Selection::ImageUrls => INLINE_REMOTE_IMAGE_URLS,
    }
}

fn real_source() -> Result<Fetcher<SystemResolver>, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| invalid_input(error.to_string()))?,
    );
    // LiteLLM's module-level client reads litellm.request_timeout, which the helper process leaves at 6000 seconds
    let core = CoreSettings {
        request_timeout_seconds: 6000.0,
        request_timeout_explicit: false,
        litellm_request_timeout: None,
        model_timeout_overrides: Vec::new(),
    };
    let client = ProxiedClient::build(base_builder(), &policy, transport_limits(&core))
        .map_err(|error| invalid_input(error.to_string()))?;
    let validator = UrlValidator::new(SystemResolver, &[], &Verification::Enabled);
    Fetcher::new(
        client,
        UserUrlFetcher {
            validator,
            validation: true,
        },
        LIMIT_MB,
        Arc::new(|| 0.0_f64),
    )
    .map_err(|error| invalid_input(error.to_string()))
}

fn inline(
    mode: Mode,
    selection: Selection,
    messages: &[Message],
    data_urls: &BTreeMap<String, String>,
) -> HelperResult {
    let runtime = runtime()?;
    let should_inline = predicate(selection);
    let source = MapSource { data_urls };
    let urls = remote_urls_to_inline(messages, &should_inline);
    let inlined = match mode {
        Mode::Sequential => {
            runtime.block_on(inline_remote_media(messages, &should_inline, &source))
        }
        Mode::Concurrent => {
            runtime.block_on(async_inline_remote_media(messages, &should_inline, &source))
        }
    }
    .map_err(|error| media_error(&error))?;
    Ok(json!({"messages": inlined, "urls": urls}))
}

fn fetch(messages: &[Message]) -> HelperResult {
    let runtime = runtime()?;
    let source = real_source()?;
    let inlined = runtime
        .block_on(inline_remote_media(
            messages,
            &INLINE_EVERY_REMOTE_URL,
            &source,
        ))
        .map_err(|error| media_error(&error))?;
    Ok(json!({"messages": inlined}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Inline {
                mode,
                should_inline,
                messages,
                data_urls,
            } => inline(*mode, *should_inline, messages, data_urls),
            Input::Fetch { messages } => fetch(messages),
            Input::GetBase64Str { values } => Ok(json!(
                values
                    .iter()
                    .map(|value| get_base64_str(value))
                    .collect::<Vec<_>>()
            )),
            Input::IsBase64Encoded { values } => Ok(json!(
                values
                    .iter()
                    .map(|value| is_base64_encoded(value))
                    .collect::<Vec<_>>()
            )),
        }
    }
}

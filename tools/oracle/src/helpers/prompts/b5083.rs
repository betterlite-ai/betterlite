use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_prompts::media_fetch::cache::ImageCache;
use betterlite_prompts::media_fetch::fetcher::Fetcher;
use betterlite_prompts::media_fetch::response::{
    BodyReadError, ImageResponse, ProcessError, process_image_response,
};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::helpers::{Helper, HelperError, HelperResult};

// LiteLLM's default MAX_IMAGE_URL_DOWNLOAD_SIZE_MB; the cases never set the variable
const LIMIT_MB: f64 = 50.0;

#[derive(Debug, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ReadResponse {
        status: u16,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        body_base64: Option<String>,
        #[serde(default)]
        body_repeat: Option<BodyRepeat>,
        url: String,
    },
    Convert {
        url: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyRepeat {
    byte: u8,
    count: usize,
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

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut map = HeaderMap::new();
    for (header_name, header_value) in headers {
        let name = HeaderName::try_from(header_name.as_str())
            .map_err(|error| invalid_input(error.to_string()))?;
        let value = HeaderValue::try_from(header_value.as_str())
            .map_err(|error| invalid_input(error.to_string()))?;
        map.insert(name, value);
    }
    Ok(map)
}

fn stream_of(body: &[u8]) -> stream::BoxStream<'static, Result<Vec<u8>, HttpError>> {
    let chunks: Vec<Result<Vec<u8>, HttpError>> =
        body.chunks(8192).map(|chunk| Ok(chunk.to_vec())).collect();
    stream::iter(chunks).boxed()
}

fn body_of(
    body_base64: Option<&str>,
    body_repeat: Option<BodyRepeat>,
) -> Result<Vec<u8>, Box<HelperError>> {
    match (body_base64, body_repeat) {
        (Some(encoded), None) => BASE64
            .decode(encoded)
            .map_err(|error| invalid_input(error.to_string())),
        (None, Some(repeat)) => Ok(vec![repeat.byte; repeat.count]),
        (None, None) => Ok(Vec::new()),
        (Some(_), Some(_)) => Err(invalid_input(
            "body_base64 and body_repeat cannot both be set".to_owned(),
        )),
    }
}

#[derive(Clone, Copy)]
struct Reading<'input> {
    status: u16,
    headers: &'input BTreeMap<String, String>,
    body_base64: Option<&'input str>,
    body_repeat: Option<BodyRepeat>,
    url: &'input str,
}

fn read_response(reading: Reading<'_>, runtime: &Runtime) -> HelperResult {
    let body = body_of(reading.body_base64, reading.body_repeat)?;
    let response = ImageResponse {
        status: reading.status,
        headers: header_map(reading.headers)?,
        body: stream_of(&body),
    };
    let cache =
        ImageCache::new(Arc::new(|| 0.0_f64)).map_err(|error| invalid_input(error.to_string()))?;
    match runtime.block_on(process_image_response(
        response,
        reading.url,
        LIMIT_MB,
        &cache,
    )) {
        Ok(data_url) => Ok(Value::String(data_url)),
        Err(ProcessError::Image(error) | ProcessError::Body(BodyReadError::TooLarge(error))) => {
            Err(Box::new(HelperError {
                class: "ImageFetchError".to_owned(),
                status: Some(400),
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            }))
        }
        Err(other) => Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: other.to_string(),
        })),
    }
}

fn convert(url: &str, runtime: &Runtime) -> HelperResult {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| invalid_input(error.to_string()))?,
    );
    let client = ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT)
        .map_err(|error| invalid_input(error.to_string()))?;
    let validator = UrlValidator::new(SystemResolver, &[], &Verification::Enabled);
    let fetcher = Fetcher::new(
        client,
        UserUrlFetcher {
            validator,
            validation: true,
        },
        LIMIT_MB,
        Arc::new(|| 0.0_f64),
    )
    .map_err(|error| invalid_input(error.to_string()))?;
    match runtime.block_on(fetcher.convert(url)) {
        Ok(data_url) => Ok(Value::String(data_url)),
        Err(error) => Err(Box::new(HelperError {
            class: "ImageFetchError".to_owned(),
            status: Some(400),
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| invalid_input(error.to_string()))?;
        match input {
            Input::ReadResponse {
                status,
                headers,
                body_base64,
                body_repeat,
                url,
            } => read_response(
                Reading {
                    status: *status,
                    headers,
                    body_base64: body_base64.as_deref(),
                    body_repeat: *body_repeat,
                    url,
                },
                &runtime,
            ),
            Input::Convert { url } => convert(url, &runtime),
        }
    }
}

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_prompts::bedrock_images::error::BedrockImageError;
use betterlite_prompts::bedrock_images::parse::{parse_content_type, parse_mime_type};
use betterlite_prompts::bedrock_images::processor::process_image;
use betterlite_prompts::bedrock_images::remote::{
    RemoteImage, TRANSPORT_LIMITS, post_call_image_processing,
};
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
    Process {
        image_url: String,
        #[serde(default)]
        format: Option<String>,
    },
    ParseContentType {
        content_type: String,
    },
    ParseMimeType {
        base64_data: String,
    },
    PostCall {
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        body_base64: String,
        image_url: String,
    },
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

fn runtime() -> Result<Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| invalid_input(error.to_string()))
}

fn remote() -> Result<RemoteImage<SystemResolver>, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| invalid_input(error.to_string()))?,
    );
    let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS)
        .map_err(|error| invalid_input(error.to_string()))?;
    let validator = UrlValidator::new(SystemResolver, &[], &Verification::Enabled);
    Ok(RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator,
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: LIMIT_MB,
    })
}

fn helper_error(error: &BedrockImageError) -> Box<HelperError> {
    let class = match error {
        BedrockImageError::Url(_) => "SSRFError",
        BedrockImageError::HttpStatus { .. } => "HTTPStatusError",
        BedrockImageError::Deadline => "TimeoutException",
        BedrockImageError::UnsupportedImageType
        | BedrockImageError::DataUrlParts
        | BedrockImageError::MissingMediaSubtype { .. }
        | BedrockImageError::MissingFormatSubtype { .. }
        | BedrockImageError::UnsupportedImageFormat { .. }
        | BedrockImageError::NoSupportedExtensions { .. }
        | BedrockImageError::Metadata(_)
        | BedrockImageError::Body(_) => "ValueError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn process(image_url: &str, format: Option<&str>) -> HelperResult {
    let runtime = runtime()?;
    let remote = remote()?;
    let block = runtime
        .block_on(process_image(image_url, format, &remote))
        .map_err(|error| helper_error(&error))?;
    serde_json::to_value(&block).map_err(|error| invalid_input(error.to_string()))
}

fn post_call(
    headers: &BTreeMap<String, String>,
    body_base64: &str,
    image_url: &str,
) -> HelperResult {
    let mut parsed = HeaderMap::new();
    for (name, value) in headers {
        parsed.append(
            HeaderName::try_from(name.as_str())
                .map_err(|error| invalid_input(error.to_string()))?,
            HeaderValue::try_from(value.as_str())
                .map_err(|error| invalid_input(error.to_string()))?,
        );
    }
    let body = BASE64
        .decode(body_base64)
        .map_err(|error| invalid_input(error.to_string()))?;
    let (base64, content_type) = post_call_image_processing(image_url, &parsed, &body)
        .map_err(|error| helper_error(&error))?;
    Ok(Value::Array(vec![
        Value::String(base64),
        Value::String(content_type),
    ]))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Process { image_url, format } => process(image_url, format.as_deref()),
            Input::ParseContentType { content_type } => {
                Ok(Value::String(parse_content_type(content_type)))
            }
            Input::ParseMimeType { base64_data } => Ok(parse_mime_type(base64_data)
                .map_or(Value::Null, |mime_type| Value::String(mime_type.to_owned()))),
            Input::PostCall {
                headers,
                body_base64,
                image_url,
            } => post_call(headers, body_base64, image_url),
        }
    }
}

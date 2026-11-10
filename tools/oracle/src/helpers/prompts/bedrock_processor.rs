use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use betterlite_prompts::bedrock_processor::conversation::{Conversion, converse_messages};
use betterlite_prompts::bedrock_processor::error::ProcessorError;
use betterlite_prompts::bedrock_tool_results::error::BedrockToolResultsError;
use betterlite_prompts::bedrock_tool_results::invoke::CachePoints;
use serde::Deserialize;
use tokio::runtime::Builder;

use crate::helpers::{Helper, HelperError, HelperResult};

// LiteLLM's default MAX_IMAGE_URL_DOWNLOAD_SIZE_MB; the cases never set the variable
const LIMIT_MB: f64 = 50.0;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Convert(ConversionInput),
    ConvertTwice(ConversionInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConversionInput {
    messages: Vec<Json>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    user_continue_message: Option<Json>,
    #[serde(default)]
    assistant_continue_message: Option<Json>,
    #[serde(default)]
    modify_params: bool,
}

// cases that set cache_control leave model unset, where LiteLLM always accepts a cache point
struct AcceptingCachePoints;

impl CachePoints for AcceptingCachePoints {
    fn cache_point_block(&self, _cache_control: &Json, _model: Option<&str>) -> Option<Object> {
        Some(Object::from_iter([(
            "type".to_owned(),
            Json::Text("default".to_owned()),
        )]))
    }
}

fn failure<Source>(class: &str, source: &Source) -> Box<HelperError>
where
    Source: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}

fn processor_failure(error: &ProcessorError) -> Box<HelperError> {
    // LiteLLM raises BadRequestError for these and lets every other conversion failure through as a ValueError
    let bad_request = matches!(
        error,
        ProcessorError::NoMessages
            | ProcessorError::UnsupportedRole { .. }
            | ProcessorError::RoleNotText { .. }
            | ProcessorError::FileObjectMissing { .. }
            | ProcessorError::FileWithoutMedia { .. }
            | ProcessorError::ToolInvoke { .. }
    );
    let file_failure = matches!(
        error,
        ProcessorError::ToolResult { source }
            if matches!(source, BedrockToolResultsError::FileWithoutMedia { .. })
    );
    Box::new(HelperError {
        class: "BedrockProcessor".to_owned(),
        status: (bad_request || file_failure).then_some(400),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn converted(input: &ConversionInput, remote: &RemoteImage<SystemResolver>) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("RuntimeError", &error))?;
    let cache_points = AcceptingCachePoints;
    let conversion = Conversion {
        model: input.model.as_deref(),
        cache_points: &cache_points,
        remote,
        user_continue_message: input.user_continue_message.as_ref(),
        assistant_continue_message: input.assistant_continue_message.as_ref(),
        modify_params: input.modify_params,
    };
    let new_id = || "call_generated".to_owned();
    let messages = runtime
        .block_on(converse_messages(&input.messages, &conversion, &new_id))
        .map_err(|error| processor_failure(&error))?;
    serde_json::to_value(&messages).map_err(|error| failure("SerializationError", &error))
}

fn convert_once(input: &ConversionInput) -> HelperResult {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| failure("ProxyError", &error))?,
    );
    let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS)
        .map_err(|error| failure("TransportError", &error))?;
    let validator = UrlValidator::new(SystemResolver, &[], &Verification::Enabled);
    let remote = RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator,
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: LIMIT_MB,
    };
    converted(input, &remote)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Convert(call) => convert_once(call),
            Input::ConvertTwice(call) => Ok(serde_json::Value::Array(vec![
                convert_once(call)?,
                convert_once(call)?,
            ])),
        }
    }
}

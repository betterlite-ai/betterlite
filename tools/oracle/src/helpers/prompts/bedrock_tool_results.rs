use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use betterlite_prompts::bedrock_tool_results::error::BedrockToolResultsError;
use betterlite_prompts::bedrock_tool_results::ids::sanitize_bedrock_tool_use_id;
use betterlite_prompts::bedrock_tool_results::invoke::{CachePoints, tool_call_invoke_blocks};
use betterlite_prompts::bedrock_tool_results::result::to_bedrock_tool_result;
use betterlite_prompts::message_content::Message;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{Helper, HelperError, HelperResult};

// LiteLLM's default MAX_IMAGE_URL_DOWNLOAD_SIZE_MB; the cases never set the variable
const LIMIT_MB: f64 = 50.0;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ToolCallInvoke {
        tool_calls: Vec<Json>,
        #[serde(default)]
        model: Option<String>,
    },
    ToolCallResult {
        message: Message,
        #[serde(default)]
        id: String,
    },
    SanitizeToolUseIds {
        values: Vec<String>,
    },
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

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error("RuntimeError", error.to_string()))
}

fn remote() -> Result<RemoteImage<SystemResolver>, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| helper_error("ProxyError", error.to_string()))?,
    );
    let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS)
        .map_err(|error| helper_error("TransportError", error.to_string()))?;
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

fn conversion_error(
    class: &str,
    status: Option<u16>,
    error: &BedrockToolResultsError,
) -> Box<HelperError> {
    let mut failure = helper_error(class, error.to_string());
    failure.status = status;
    failure
}

fn ok_value<Payload>(payload: &Payload) -> HelperResult
where
    Payload: Serialize,
{
    serde_json::to_value(payload)
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ToolCallInvoke { tool_calls, model } => {
                // LiteLLM wraps every failure of the invoke conversion in a BadRequestError
                let blocks =
                    tool_call_invoke_blocks(tool_calls, model.as_deref(), &AcceptingCachePoints)
                        .map_err(|error| conversion_error("BadRequestError", Some(400), &error))?;
                ok_value(&blocks)
            }
            Input::ToolCallResult { message, id } => {
                let runtime = runtime()?;
                let remote = remote()?;
                let ids = || id.clone();
                let block = runtime
                    .block_on(to_bedrock_tool_result(message, &remote, &ids))
                    .map_err(|error| {
                        // the result conversion raises its file failure as a BadRequestError and lets the rest through
                        let status =
                            matches!(&error, BedrockToolResultsError::FileWithoutMedia { .. })
                                .then_some(400);
                        conversion_error("BedrockToolResultError", status, &error)
                    })?;
                ok_value(&block)
            }
            Input::SanitizeToolUseIds { values } => {
                let sanitized: Vec<String> = values
                    .iter()
                    .map(|value| sanitize_bedrock_tool_use_id(value))
                    .collect();
                ok_value(&sanitized)
            }
        }
    }
}

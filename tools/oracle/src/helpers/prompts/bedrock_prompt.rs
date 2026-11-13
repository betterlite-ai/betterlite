use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use betterlite_prompts::bedrock::assembly::{
    BedrockPrompt, PromptDeps, PromptSettings, bedrock_prompt,
};
use betterlite_prompts::bedrock::error::PromptError;
use betterlite_prompts::bedrock::flags::MessageCachePoints;
use betterlite_prompts::bedrock::input::PromptBedrockInput;
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use betterlite_prompts::bedrock_processor::error::ProcessorError;
use betterlite_prompts::bedrock_tool_results::error::BedrockToolResultsError;
use betterlite_prompts::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use tokio::runtime::Builder;

use crate::helpers::{Helper, HelperError, HelperResult};

const GENERATED_ID: &str = "generated-id";
// LiteLLM's default MAX_IMAGE_URL_DOWNLOAD_SIZE_MB; the cases never set the variable
const LIMIT_MB: f64 = 50.0;

#[derive(Debug, Clone, Copy)]
struct RefusingResolver;

#[async_trait]
impl Resolve for RefusingResolver {
    async fn resolve(&self, host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("the case resolves no host, including {host}"),
        ))
    }
}

fn failure(status: Option<u16>, error: &dyn std::fmt::Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BedrockPrompt".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// LiteLLM raises BadRequestError for these conversion failures and lets every other one escape as a ValueError
fn prompt_failure(error: &PromptError) -> Box<HelperError> {
    let status = match error {
        PromptError::Processor(
            ProcessorError::NoMessages
            | ProcessorError::UnsupportedRole { .. }
            | ProcessorError::RoleNotText { .. }
            | ProcessorError::FileObjectMissing { .. }
            | ProcessorError::FileWithoutMedia { .. }
            | ProcessorError::ToolInvoke { .. }
            | ProcessorError::ToolResult {
                source: BedrockToolResultsError::FileWithoutMedia { .. },
            },
        ) => Some(400),
        PromptError::Processor(_) | PromptError::Tools(_) => None,
    };
    failure(status, error)
}

fn transport() -> Result<RemoteImage<RefusingResolver>, Box<HelperError>> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .map_err(|error| failure(None, &error))?,
    );
    let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS)
        .map_err(|error| failure(None, &error))?;
    let validator = UrlValidator::new(RefusingResolver, &[], &Verification::Enabled);
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

fn converted(input: &PromptBedrockInput) -> Result<BedrockPrompt, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure(None, &error))?;
    let remote = transport()?;
    let cache_points = MessageCachePoints {
        support: CachePointSupport::from(input.capabilities.cache_points),
    };
    let settings = PromptSettings {
        model: input.model.as_deref(),
        capabilities: BedrockToolCapabilities::from(input.capabilities),
        user_continue_message: input.user_continue_message.as_ref(),
        assistant_continue_message: input.assistant_continue_message.as_ref(),
        modify_params: input.modify_params,
    };
    let new_id = || GENERATED_ID.to_owned();
    let deps = PromptDeps {
        cache_points: &cache_points,
        remote: &remote,
        new_id: &new_id,
    };
    runtime
        .block_on(bedrock_prompt(
            &input.messages,
            &input.tools,
            &settings,
            &deps,
        ))
        .map_err(|error| prompt_failure(&error))
}

fn answer(prompt: &BedrockPrompt) -> HelperResult {
    let messages = serde_json::to_value(&prompt.messages).map_err(|error| failure(None, &error))?;
    let tools = serde_json::to_value(&prompt.tools).map_err(|error| failure(None, &error))?;
    let names = serde_json::to_value(&prompt.names).map_err(|error| failure(None, &error))?;
    Ok(serde_json::json!({
        "messages": messages,
        "tools": tools,
        "tool_name_mappings": names,
    }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = PromptBedrockInput;

    fn run(input: &Self::Input) -> HelperResult {
        let prompt = converted(input)?;
        answer(&prompt)
    }
}

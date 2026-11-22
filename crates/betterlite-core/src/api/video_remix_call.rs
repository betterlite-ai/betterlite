pub mod config;
pub mod error;
mod prepare;
pub(crate) mod shared;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::config::{
    DEFAULT_PROVIDER, DEFAULT_TIMEOUT_SECONDS, VideoConfigLookup, VideoOperation, provider_config,
};
use self::error::{VideoCallError, VideoCallFailure, failure};
use self::prepare::{PreparedRemix, prepare_remix};
use self::shared::mock_answer;
use crate::api::video_records::VideoObject;
use crate::api::video_remix_list_transport::client::{VideoClient, VideoClientSource};
use crate::api::video_remix_list_transport::records::Params;
use crate::api::video_remix_list_transport::{
    Handled, LogContext, RemixCall, VideoRemixListTransport,
};

// Every keyword video_remix takes, typed the way a caller passes it
#[derive(Clone, Debug)]
pub struct VideoRemixCall {
    pub video_id: String,
    pub prompt: String,
    pub custom_llm_provider: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub extra_headers: Option<IndexMap<String, String>>,
    // video_remix never forwards this, and the keyword is still accepted
    pub extra_query: Option<Params>,
    pub extra_body: Option<Params>,
    pub mock_response: Option<Json>,
    pub litellm_metadata: Option<IndexMap<String, JsonValue>>,
    pub litellm_call_id: Option<String>,
    pub user: Option<String>,
    // the signature's `timeout: Optional[float] = 600`; None is what a caller passes explicitly
    pub timeout: Option<f64>,
    // every other keyword, kept for GenericLiteLLMParams and the logging update
    pub extra: IndexMap<String, Json>,
}

impl Default for VideoRemixCall {
    fn default() -> Self {
        Self {
            video_id: String::new(),
            prompt: String::new(),
            custom_llm_provider: None,
            api_key: None,
            api_base: None,
            api_version: None,
            extra_headers: None,
            extra_query: None,
            extra_body: None,
            mock_response: None,
            litellm_metadata: None,
            litellm_call_id: None,
            user: None,
            timeout: Some(DEFAULT_TIMEOUT_SECONDS),
            extra: IndexMap::new(),
        }
    }
}

// The collaborators one call writes into: B3332's tag lookup, B247's logging state, the pre/post environment and B185's redactor
pub struct VideoRemixDeps<'deps, 'env> {
    pub configs: &'deps dyn VideoConfigLookup,
    pub logging: &'deps mut LoggingState,
    pub env: &'deps PrePostEnv<'env>,
    pub redactor: &'deps Redactor,
    // B198's REQUEST_TIMEOUT, the fallback for a call with no usable timeout
    pub request_timeout_seconds: f64,
}

// video_remix: the mock short circuit, then the provider, the config and the sync handler
pub fn video_remix<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoRemixDeps<'_, '_>,
    call: &VideoRemixCall,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoObject, VideoCallFailure>
where
    Source: VideoClientSource,
{
    let VideoRemixDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    } = deps;
    let argument_provider = call.custom_llm_provider.as_deref().unwrap_or_default();
    if let Some(object) = decode_mock(call, argument_provider, redactor)? {
        return Ok(object);
    }
    let provider = remix_provider(call);
    let ready = VideoRemixDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    };
    run_remix(transport, ready, call, &provider, client)
        .map_err(|error| failure(error, &provider, redactor))
}

// avideo_remix: the same body through the async handler, awaited here instead of in an executor
pub async fn avideo_remix<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoRemixDeps<'_, '_>,
    call: &VideoRemixCall,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoObject, VideoCallFailure>
where
    Source: VideoClientSource,
{
    let VideoRemixDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    } = deps;
    let argument_provider = call.custom_llm_provider.as_deref().unwrap_or_default();
    if let Some(object) = decode_mock(call, argument_provider, redactor)? {
        return Ok(object);
    }
    let provider = remix_provider(call);
    let ready = VideoRemixDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    };
    run_remix_async(transport, ready, call, &provider, client)
        .await
        .map_err(|error| failure(error, &provider, redactor))
}

// VideoObject(**json.loads(mock_response)) or VideoObject(**mock_response), before any provider work
fn decode_mock(
    call: &VideoRemixCall,
    provider: &str,
    redactor: &Redactor,
) -> Result<Option<VideoObject>, VideoCallFailure> {
    mock_answer(call.mock_response.as_ref(), "a video object")
        .map_err(|error| failure(VideoCallError::Mock(error), provider, redactor))
}

// custom_llm_provider or decode_video_id_with_provider(video_id)["custom_llm_provider"] or "openai"
fn remix_provider(call: &VideoRemixCall) -> String {
    match &call.custom_llm_provider {
        Some(provider) => provider.clone(),
        None => decode_video_id_with_provider(&call.video_id)
            .custom_llm_provider
            .filter(|provider| !provider.is_empty())
            .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned()),
    }
}

fn run_remix<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoRemixDeps<'_, '_>,
    call: &VideoRemixCall,
    provider: &str,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoObject, VideoCallError>
where
    Source: VideoClientSource,
{
    let VideoRemixDeps {
        configs,
        logging,
        env,
        redactor: _,
        request_timeout_seconds,
    } = deps;
    let config = provider_config(configs, provider, VideoOperation::Remix)?;
    let prepared = prepare_remix(logging, call, provider, request_timeout_seconds)?;
    let handled = transport.remix(
        config,
        remix_wire_call(call, provider, &prepared, client),
        LogContext {
            state: logging,
            env,
        },
        false,
    )?;
    match handled {
        Handled::Done(object) => Ok(object),
        Handled::Pending(_pending) => Err(VideoCallError::UnexpectedPending {
            operation: VideoOperation::Remix,
        }),
    }
}

async fn run_remix_async<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoRemixDeps<'_, '_>,
    call: &VideoRemixCall,
    provider: &str,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoObject, VideoCallError>
where
    Source: VideoClientSource,
{
    let VideoRemixDeps {
        configs,
        logging,
        env,
        redactor: _,
        request_timeout_seconds,
    } = deps;
    let config = provider_config(configs, provider, VideoOperation::Remix)?;
    let prepared = prepare_remix(logging, call, provider, request_timeout_seconds)?;
    let handled = transport.remix(
        config,
        remix_wire_call(call, provider, &prepared, client),
        LogContext {
            state: logging,
            env,
        },
        true,
    )?;
    match handled {
        Handled::Pending(pending) => Ok(pending.await?),
        Handled::Done(object) => Ok(object),
    }
}

// video_remix passes no api_key to the handler: the key reaches validate_environment through litellm_params
fn remix_wire_call<'call, Source>(
    call: &'call VideoRemixCall,
    provider: &'call str,
    prepared: &'call PreparedRemix,
    client: Option<VideoClient<'call, Source>>,
) -> RemixCall<'call, Source>
where
    Source: VideoClientSource,
{
    RemixCall {
        video_id: &call.video_id,
        prompt: &call.prompt,
        custom_llm_provider: provider,
        litellm_params: &prepared.litellm_params,
        client,
        extra_headers: call.extra_headers.as_ref(),
        extra_body: call.extra_body.as_ref(),
        timeout: Some(prepared.timeout),
        api_key: None,
    }
}

#[cfg(test)]
#[path = "video_remix_call/tests/support.rs"]
pub(crate) mod support;

#[cfg(test)]
#[path = "video_remix_call/tests/remix_timeout.rs"]
mod remix_timeout;

#[cfg(test)]
#[path = "video_remix_call/tests/remix_provider.rs"]
mod remix_provider;

#[cfg(test)]
#[path = "video_remix_call/tests/mock_response.rs"]
mod mock_response;

#[cfg(test)]
#[path = "video_remix_call/tests/call_type.rs"]
mod call_type;

#[cfg(test)]
#[path = "video_remix_call/tests/async_wrapper.rs"]
mod async_wrapper;

#[cfg(test)]
#[path = "video_remix_call/tests/unsupported_provider.rs"]
mod unsupported_provider;

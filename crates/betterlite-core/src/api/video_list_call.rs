mod prepare;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::prepare::{PreparedList, prepare_list};
use crate::api::video_records::VideoObject;
use crate::api::video_remix_call::config::{
    DEFAULT_PROVIDER, DEFAULT_TIMEOUT_SECONDS, VideoConfigLookup, VideoOperation, provider_config,
};
use crate::api::video_remix_call::error::{VideoCallError, VideoCallFailure, failure};
use crate::api::video_remix_call::shared::mock_answer;
use crate::api::video_remix_list_transport::client::{VideoClient, VideoClientSource};
use crate::api::video_remix_list_transport::records::Params;
use crate::api::video_remix_list_transport::{
    Handled, ListCall, LogContext, VideoRemixListTransport,
};

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum VideoListReply {
    Objects(Vec<VideoObject>),
    Page(Params),
}

#[derive(Clone, Debug)]
pub struct VideoListCall {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub order: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub extra_query: Option<Params>,
    pub extra_body: Option<Params>,
    pub mock_response: Option<Json>,
    pub litellm_metadata: Option<IndexMap<String, JsonValue>>,
    pub litellm_call_id: Option<String>,
    pub user: Option<String>,
    // the signature's `timeout: Optional[float] = 600`; None is what a caller passes explicitly
    pub timeout: Option<f64>,
    pub extra: IndexMap<String, Json>,
}

impl Default for VideoListCall {
    fn default() -> Self {
        Self {
            after: None,
            limit: None,
            order: None,
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

pub struct VideoListDeps<'deps, 'env> {
    pub configs: &'deps dyn VideoConfigLookup,
    pub logging: &'deps mut LoggingState,
    pub env: &'deps PrePostEnv<'env>,
    pub redactor: &'deps Redactor,
    // B198's REQUEST_TIMEOUT, the fallback for a call with no usable timeout
    pub request_timeout_seconds: f64,
}

pub fn video_list<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoListDeps<'_, '_>,
    call: &VideoListCall,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoListReply, VideoCallFailure>
where
    Source: VideoClientSource,
{
    let VideoListDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    } = deps;
    let argument_provider = call.custom_llm_provider.as_deref().unwrap_or_default();
    if let Some(objects) = decode_mock(call, argument_provider, redactor)? {
        return Ok(VideoListReply::Objects(objects));
    }
    let provider = sync_provider(call);
    let ready = VideoListDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    };
    run_list(transport, ready, call, &provider, client)
        .map_err(|error| failure(error, &provider, redactor))
}

// avideo_list: get_llm_provider(model="") before the mock, then the same body through the async handler
pub async fn avideo_list<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoListDeps<'_, '_>,
    resolution: &ResolutionContext<'_>,
    call: &VideoListCall,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoListReply, VideoCallFailure>
where
    Source: VideoClientSource,
{
    let VideoListDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    } = deps;
    let provider = match &call.custom_llm_provider {
        Some(provider) => provider.clone(),
        None => inferred_provider(resolution).map_err(|error| failure(error, "", redactor))?,
    };
    if let Some(objects) = decode_mock(call, &provider, redactor)? {
        return Ok(VideoListReply::Objects(objects));
    }
    let ready = VideoListDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    };
    run_list_async(transport, ready, call, &provider, client)
        .await
        .map_err(|error| failure(error, &provider, redactor))
}

// [VideoObject(**item) for item in json.loads(mock_response) or mock_response], before any provider work
fn decode_mock(
    call: &VideoListCall,
    provider: &str,
    redactor: &Redactor,
) -> Result<Option<Vec<VideoObject>>, VideoCallFailure> {
    mock_answer(call.mock_response.as_ref(), "a video list")
        .map_err(|error| failure(VideoCallError::Mock(error), provider, redactor))
}

// video_list defaults to openai and never infers from the model; an explicit provider is kept as given
fn sync_provider(call: &VideoListCall) -> String {
    call.custom_llm_provider
        .clone()
        .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
}

// avideo_list's get_llm_provider(model="", api_base=None): an empty model with no declaration resolves nothing
fn inferred_provider(resolution: &ResolutionContext<'_>) -> Result<String, VideoCallError> {
    let request = ResolutionRequest {
        model: "",
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    let resolved = resolve(&request, resolution)?;
    let provider = resolved.provider.slug().to_owned();
    Ok(provider)
}

fn run_list<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoListDeps<'_, '_>,
    call: &VideoListCall,
    provider: &str,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoListReply, VideoCallError>
where
    Source: VideoClientSource,
{
    let VideoListDeps {
        configs,
        logging,
        env,
        redactor: _,
        request_timeout_seconds,
    } = deps;
    let config = provider_config(configs, provider, VideoOperation::List)?;
    let prepared = prepare_list(logging, call, provider, request_timeout_seconds)?;
    let handled = transport.list(
        config,
        list_wire_call(call, provider, &prepared, client),
        LogContext {
            state: logging,
            env,
        },
        false,
    )?;
    match handled {
        Handled::Done(params) => Ok(VideoListReply::Page(params)),
        Handled::Pending(_pending) => Err(VideoCallError::UnexpectedPending {
            operation: VideoOperation::List,
        }),
    }
}

async fn run_list_async<Source>(
    transport: &VideoRemixListTransport<Source>,
    deps: VideoListDeps<'_, '_>,
    call: &VideoListCall,
    provider: &str,
    client: Option<VideoClient<'_, Source>>,
) -> Result<VideoListReply, VideoCallError>
where
    Source: VideoClientSource,
{
    let VideoListDeps {
        configs,
        logging,
        env,
        redactor: _,
        request_timeout_seconds,
    } = deps;
    let config = provider_config(configs, provider, VideoOperation::List)?;
    let prepared = prepare_list(logging, call, provider, request_timeout_seconds)?;
    let handled = transport.list(
        config,
        list_wire_call(call, provider, &prepared, client),
        LogContext {
            state: logging,
            env,
        },
        true,
    )?;
    match handled {
        Handled::Pending(pending) => Ok(VideoListReply::Page(pending.await?)),
        Handled::Done(params) => Ok(VideoListReply::Page(params)),
    }
}

// video_list passes no api_key to the handler: validate_environment there sees only the params' key
fn list_wire_call<'call, Source>(
    call: &'call VideoListCall,
    provider: &'call str,
    prepared: &'call PreparedList,
    client: Option<VideoClient<'call, Source>>,
) -> ListCall<'call, Source>
where
    Source: VideoClientSource,
{
    ListCall {
        after: call.after.as_deref(),
        limit: call.limit,
        order: call.order.as_deref(),
        custom_llm_provider: provider,
        litellm_params: &prepared.litellm_params,
        client,
        extra_headers: call.extra_headers.as_ref(),
        extra_query: call.extra_query.as_ref(),
        timeout: Some(prepared.timeout),
        api_key: None,
    }
}

#[cfg(test)]
#[path = "video_list_call/tests/sync_default_openai.rs"]
mod sync_default_openai;

#[cfg(test)]
#[path = "video_list_call/tests/async_inference.rs"]
mod async_inference;

#[cfg(test)]
#[path = "video_list_call/tests/list_timeout.rs"]
mod list_timeout;

#[cfg(test)]
#[path = "video_list_call/tests/mock_response.rs"]
mod mock_response;

#[cfg(test)]
#[path = "video_list_call/tests/pagination_params.rs"]
mod pagination_params;

#[cfg(test)]
#[path = "video_list_call/tests/call_type.rs"]
mod call_type;

#[cfg(test)]
#[path = "video_list_call/tests/async_wrapper.rs"]
mod async_wrapper;

#[cfg(test)]
#[path = "video_list_call/tests/unsupported_provider.rs"]
mod unsupported_provider;

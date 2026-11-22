mod prepare;

use std::future::Future;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::prepare::{PreparedList, prepare_list};
use crate::api::video_records::VideoObject;
use crate::api::video_remix_call::config::{
    DEFAULT_PROVIDER, DEFAULT_TIMEOUT_SECONDS, VideoOperation, provider_config,
};
use crate::api::video_remix_call::error::{VideoCallError, VideoCallFailure, failure};
use crate::api::video_remix_call::shared::{VideoCallDeps, mock_answer};
use crate::api::video_remix_list_transport::client::{VideoClient, VideoClientSource};
use crate::api::video_remix_list_transport::error::VideoRemixListTransportError;
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

// Everything one list entry hands the transport: the pool, the collaborators, the call and its client
pub struct ListRequest<'call, 'deps, 'env, Source: VideoClientSource> {
    pub transport: &'deps VideoRemixListTransport<Source>,
    pub deps: VideoCallDeps<'deps, 'env>,
    pub call: &'call VideoListCall,
    pub client: Option<VideoClient<'call, Source>>,
}

// video_list: the mock short circuit, then openai unless the caller named a provider
pub fn video_list<Source: VideoClientSource>(
    request: ListRequest<'_, '_, '_, Source>,
) -> Result<VideoListReply, VideoCallFailure> {
    let provider = sync_provider(request.call);
    let argument = request.call.custom_llm_provider.as_deref();
    list_begin(request, argument.unwrap_or_default(), provider, false)?.finish()
}

// avideo_list: get_llm_provider(model="") before the mock, then the async handler awaited here
pub async fn avideo_list<Source: VideoClientSource>(
    request: ListRequest<'_, '_, '_, Source>,
    resolution: &ResolutionContext<'_>,
) -> Result<VideoListReply, VideoCallFailure> {
    let provider = match &request.call.custom_llm_provider {
        Some(provider) => provider.clone(),
        None => inferred_provider(resolution)
            .map_err(|error| failure(error, "", request.deps.redactor))?,
    };
    list_begin(request, &provider, provider.clone(), true)?
        .finish_async()
        .await
}

// The two answers a list entry can carry: the mock's objects, or the handler's answer
enum ListAnswer<Pending> {
    Objects(Vec<VideoObject>),
    Handled(Handled<Params, Pending>),
}

// What a list entry has after list_begin: its answer, the provider and the redactor an error prints with
struct ListBegin<'redactor, Pending> {
    answer: ListAnswer<Pending>,
    provider: String,
    redactor: &'redactor Redactor,
}

impl<'redactor, Pending> ListBegin<'redactor, Pending>
where
    Pending: Future<Output = Result<Params, VideoRemixListTransportError>>,
{
    // the sync entry's answer: the mock's objects, or the params the handler answered in place
    fn finish(self) -> Result<VideoListReply, VideoCallFailure> {
        match self.answer {
            ListAnswer::Objects(objects) => Ok(VideoListReply::Objects(objects)),
            ListAnswer::Handled(handled) => list_answer(handled, &self.provider, self.redactor),
        }
    }

    // the async entry's answer: the handler's future, awaited and mapped like a direct failure
    async fn finish_async(self) -> Result<VideoListReply, VideoCallFailure> {
        let pending = match self.answer {
            ListAnswer::Objects(objects) => return Ok(VideoListReply::Objects(objects)),
            ListAnswer::Handled(Handled::Done(params)) => {
                return Ok(VideoListReply::Page(params));
            }
            ListAnswer::Handled(Handled::Pending(pending)) => pending,
        };
        let params = pending
            .await
            .map_err(|error| failure(VideoCallError::from(error), &self.provider, self.redactor))?;
        Ok(VideoListReply::Page(params))
    }
}

// a sync handler's answer: the params page, or the failure of answering with a future instead
fn list_answer<Pending: Future<Output = Result<Params, VideoRemixListTransportError>>>(
    handled: Handled<Params, Pending>,
    provider: &str,
    redactor: &Redactor,
) -> Result<VideoListReply, VideoCallFailure> {
    match handled {
        Handled::Done(params) => Ok(VideoListReply::Page(params)),
        Handled::Pending(_pending) => Err(list_pending(provider, redactor)),
    }
}

// the mock short circuit, the provider, the config and the handler, which answers in place or with a future
fn list_begin<'call, 'deps, 'env, Source: VideoClientSource>(
    request: ListRequest<'call, 'deps, 'env, Source>,
    mock_provider: &str,
    provider: String,
    is_async: bool,
) -> Result<
    ListBegin<'deps, impl Future<Output = Result<Params, VideoRemixListTransportError>> + Send>,
    VideoCallFailure,
> {
    let redactor = request.deps.redactor;
    let mock = mock_answer(request.call.mock_response.as_ref(), "a video list")
        .map_err(|error| failure(VideoCallError::Mock(error), mock_provider, redactor))?;
    let answer = match mock {
        Some(objects) => ListAnswer::Objects(objects),
        None => {
            let VideoCallDeps {
                configs,
                logging,
                env,
                request_timeout_seconds,
                ..
            } = request.deps;
            let config = provider_config(configs, &provider, VideoOperation::List)?;
            let prepared = prepare_list(logging, request.call, &provider, request_timeout_seconds)?;
            let handled = request
                .transport
                .list(
                    config,
                    list_wire_call(request.call, &provider, &prepared, request.client),
                    LogContext {
                        state: logging,
                        env,
                    },
                    is_async,
                )
                .map_err(|error| failure(error, &provider, redactor))?;
            ListAnswer::Handled(handled)
        }
    };
    Ok(ListBegin {
        answer,
        provider,
        redactor,
    })
}

// a sync handler that answered with a future where the entry expected a direct answer
fn list_pending(provider: &str, redactor: &Redactor) -> VideoCallFailure {
    failure(
        VideoCallError::UnexpectedPending {
            operation: VideoOperation::List,
        },
        provider,
        redactor,
    )
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

#[path = "video_list_call/tests/sync_default_openai.rs"]
mod sync_default_openai;

#[path = "video_list_call/tests/async_inference.rs"]
mod async_inference;

#[path = "video_list_call/tests/list_timeout.rs"]
mod list_timeout;

#[path = "video_list_call/tests/mock_response.rs"]
mod mock_response;

#[path = "video_list_call/tests/pagination_params.rs"]
mod pagination_params;

#[path = "video_list_call/tests/call_type.rs"]
mod call_type;

#[path = "video_list_call/tests/async_wrapper.rs"]
mod async_wrapper;

#[path = "video_list_call/tests/unsupported_provider.rs"]
mod unsupported_provider;

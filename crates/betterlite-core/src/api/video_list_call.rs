mod prepare;

use std::future::Future;

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
    DEFAULT_PROVIDER, DEFAULT_TIMEOUT_SECONDS, VideoOperation, provider_config,
};
use crate::api::video_remix_call::error::{VideoCallError, VideoCallFailure, failure};
use crate::api::video_remix_call::shared::{VideoCallDeps, mock_answer};
use crate::api::video_remix_list_transport::client::{VideoClient, VideoClientSource};
use crate::api::video_remix_list_transport::config::VideoRemixListConfig;
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

impl<Source: VideoClientSource> ListRequest<'_, '_, '_, Source> {
    // the mock short circuit, then openai unless the caller named a provider
    fn direct(self) -> Result<VideoListReply, VideoCallFailure> {
        if let Some(objects) = list_mock(self.call, self.deps.redactor)? {
            return Ok(VideoListReply::Objects(objects));
        }
        list_ready(sync_provider(self.call), self)?.direct()
    }
}

// ast-grep-ignore: no-one-line-functions - B3340 exposes LiteLLM's video_list under this public name
pub fn video_list<Source: VideoClientSource>(
    request: ListRequest<'_, '_, '_, Source>,
) -> Result<VideoListReply, VideoCallFailure> {
    request.direct()
}

// avideo_list: get_llm_provider(model="") before the mock, then the async handler awaited here
pub async fn avideo_list<Source: VideoClientSource>(
    request: ListRequest<'_, '_, '_, Source>,
    resolution: &ResolutionContext<'_>,
) -> Result<VideoListReply, VideoCallFailure> {
    let provider = match &request.call.custom_llm_provider {
        Some(data) => data.clone(),
        None => inferred_provider(resolution)
            .map_err(|error| failure(error, "", request.deps.redactor))?,
    };
    if let Some(objects) = list_mock(request.call, request.deps.redactor)? {
        return Ok(VideoListReply::Objects(objects));
    }
    list_ready(provider, request)?.awaited().await
}

// The provider and the prepared call, with every collaborator the handler borrows, held by the entry that awaits it
struct ListReady<'call, 'deps, 'env, Source: VideoClientSource> {
    transport: &'deps VideoRemixListTransport<Source>,
    config: &'deps dyn VideoRemixListConfig,
    logging: &'deps mut LoggingState,
    env: &'deps PrePostEnv<'env>,
    redactor: &'deps Redactor,
    call: &'call VideoListCall,
    client: Option<VideoClient<'call, Source>>,
    provider: String,
    prepared: PreparedList,
}

impl<Source: VideoClientSource> ListReady<'_, '_, '_, Source> {
    // the handler's answer: the params page, or a future that borrows this ready call
    fn run(
        &mut self,
        is_async: bool,
    ) -> Result<
        Handled<
            Params,
            impl Future<Output = Result<Params, VideoRemixListTransportError>> + Send + '_,
        >,
        VideoCallFailure,
    > {
        let call = list_wire_call(
            self.call,
            &self.provider,
            &self.prepared,
            self.client.take(),
        );
        let log = LogContext {
            state: &mut *self.logging,
            env: self.env,
        };
        self.transport
            .list(self.config, call, log, is_async)
            .map_err(|error| failure(error, &self.provider, self.redactor))
    }

    // a sync handler must answer directly; a future in its place is the UnexpectedPending failure
    fn direct(mut self) -> Result<VideoListReply, VideoCallFailure> {
        let Handled::Done(params) = self.run(false)? else {
            return Err(list_pending(&self.provider, self.redactor));
        };
        Ok(VideoListReply::Page(params))
    }

    // the async handler's future, awaited here
    async fn awaited(mut self) -> Result<VideoListReply, VideoCallFailure> {
        let provider = self.provider.clone();
        let redactor = self.redactor;
        match self.run(true)? {
            Handled::Done(params) => Ok(VideoListReply::Page(params)),
            Handled::Pending(pending) => pending
                .await
                .map(VideoListReply::Page)
                .map_err(|error| failure(VideoCallError::from(error), &provider, redactor)),
        }
    }
}

// the mock short circuit: LiteLLM answers from mock_response before it reads the provider's config
fn list_mock(
    call: &VideoListCall,
    redactor: &Redactor,
) -> Result<Option<Vec<VideoObject>>, VideoCallFailure> {
    let mock_provider = call.custom_llm_provider.as_deref().unwrap_or_default();
    mock_answer(call.mock_response.as_ref(), "a video list")
        .map_err(|error| failure(VideoCallError::Mock(error), mock_provider, redactor))
}

// the provider's config and the call's preparation, kept together for the handler
fn list_ready<'call, 'deps, 'env, Source: VideoClientSource>(
    provider: String,
    request: ListRequest<'call, 'deps, 'env, Source>,
) -> Result<ListReady<'call, 'deps, 'env, Source>, VideoCallFailure> {
    let ListRequest {
        transport,
        deps,
        call,
        client,
    } = request;
    let VideoCallDeps {
        configs,
        logging,
        env,
        redactor,
        request_timeout_seconds,
    } = deps;
    let config = provider_config(configs, &provider, VideoOperation::List)
        .map_err(|error| failure(error, &provider, redactor))?;
    let prepared = prepare_list(logging, call, &provider, request_timeout_seconds)
        .map_err(|error| failure(error, &provider, redactor))?;
    Ok(ListReady {
        transport,
        config,
        logging,
        env,
        redactor,
        call,
        client,
        provider,
        prepared,
    })
}

// a sync handler that answered with a future where the entry expected a direct answer
fn list_pending(provider: &str, redactor: &Redactor) -> VideoCallFailure {
    let error = VideoCallError::UnexpectedPending {
        operation: VideoOperation::List,
    };
    failure(error, provider, redactor)
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

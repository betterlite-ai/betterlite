pub mod config;
pub mod error;
mod prepare;
pub mod shared;

use std::future::Future;

use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::config::{DEFAULT_PROVIDER, DEFAULT_TIMEOUT_SECONDS, VideoOperation, provider_config};
use self::error::{REMIX_PENDING, VideoCallError, VideoCallFailure, failure};
use self::prepare::{PreparedRemix, prepare_remix};
use self::shared::{VideoCallDeps, mock_answer};
use crate::api::video_records::VideoObject;
use crate::api::video_remix_list_transport::client::{VideoClient, VideoClientSource};
use crate::api::video_remix_list_transport::config::VideoRemixListConfig;
use crate::api::video_remix_list_transport::error::VideoRemixListTransportError;
use crate::api::video_remix_list_transport::records::Params;
use crate::api::video_remix_list_transport::{
    Handled, LogContext, RemixCall, VideoRemixListTransport,
};

// both video_remix entries' answer: the object LiteLLM's handler returns
type RemixAnswer = Result<VideoObject, VideoCallFailure>;

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

// Everything one remix entry hands the transport: the pool, the collaborators, the call and its client
pub struct RemixRequest<'call, 'deps, 'env, Source: VideoClientSource> {
    pub transport: &'deps VideoRemixListTransport<Source>,
    pub deps: VideoCallDeps<'deps, 'env>,
    pub call: &'call VideoRemixCall,
    pub client: Option<VideoClient<'call, Source>>,
}

impl<Source: VideoClientSource> RemixRequest<'_, '_, '_, Source> {
    // the mock short circuit, then the handler's sync answer or the pending failure
    fn direct(self) -> Result<VideoObject, VideoCallFailure> {
        if let Some(object) = remix_mock(self.call, self.deps.redactor)? {
            return Ok(object);
        }
        RemixReady::ready(self)?.direct()
    }
}

// ast-grep-ignore: no-one-line-functions - B3340 exposes LiteLLM's video_remix under this public name
pub fn video_remix<Source>(request: RemixRequest<'_, '_, '_, Source>) -> RemixAnswer
where
    Source: VideoClientSource,
{
    request.direct()
}

pub async fn avideo_remix<Source>(request: RemixRequest<'_, '_, '_, Source>) -> RemixAnswer
where
    Source: VideoClientSource,
{
    if let Some(object) = remix_mock(request.call, request.deps.redactor)? {
        return Ok(object);
    }
    RemixReady::ready(request)?.awaited().await
}

// custom_llm_provider or decode_video_id_with_provider(video_id)["custom_llm_provider"] or "openai"
fn remix_provider(call: &VideoRemixCall) -> String {
    call.custom_llm_provider.clone().unwrap_or_else(|| {
        decode_video_id_with_provider(&call.video_id)
            .custom_llm_provider
            .filter(|provider| !provider.is_empty())
            .unwrap_or_else(|| DEFAULT_PROVIDER.to_owned())
    })
}

// The provider and the prepared call, with every collaborator the handler borrows, held by the entry that awaits it
struct RemixReady<'call, 'deps, 'env, Source: VideoClientSource> {
    transport: &'deps VideoRemixListTransport<Source>,
    config: &'deps dyn VideoRemixListConfig,
    logging: &'deps mut LoggingState,
    env: &'deps PrePostEnv<'env>,
    redactor: &'deps Redactor,
    call: &'call VideoRemixCall,
    client: Option<VideoClient<'call, Source>>,
    provider: String,
    prepared: PreparedRemix,
}

impl<'call, 'deps, 'env, Source: VideoClientSource> RemixReady<'call, 'deps, 'env, Source> {
    // the provider, its config and the call's preparation, kept together for the handler
    fn ready(request: RemixRequest<'call, 'deps, 'env, Source>) -> Result<Self, VideoCallFailure> {
        let RemixRequest {
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
        let provider = remix_provider(call);
        let config = provider_config(configs, &provider, VideoOperation::Remix)
            .map_err(|error| failure(error, &provider, redactor))?;
        let prepared = prepare_remix(logging, call, &provider, request_timeout_seconds)
            .map_err(|error| failure(error, &provider, redactor))?;
        Ok(RemixReady {
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

    // the handler's answer: the object itself, or a future that borrows this ready call
    fn run(
        &mut self,
        is_async: bool,
    ) -> Result<
        Handled<
            VideoObject,
            impl Future<Output = Result<VideoObject, VideoRemixListTransportError>> + Send + '_,
        >,
        VideoCallFailure,
    > {
        let call = remix_wire_call(
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
            .remix(self.config, call, log, is_async)
            .map_err(|error| failure(VideoCallError::from(error), &self.provider, self.redactor))
    }

    // a sync handler must answer directly; a future in its place is the UnexpectedPending failure
    fn direct(mut self) -> Result<VideoObject, VideoCallFailure> {
        let Handled::Done(object) = self.run(false)? else {
            return Err(failure(REMIX_PENDING, &self.provider, self.redactor));
        };
        Ok(object)
    }

    // the async handler's future, awaited here
    async fn awaited(mut self) -> Result<VideoObject, VideoCallFailure> {
        let provider = self.provider.clone();
        let redactor = self.redactor;
        match self.run(true)? {
            Handled::Done(object) => Ok(object),
            Handled::Pending(pending) => pending
                .await
                .map_err(|error| failure(VideoCallError::from(error), &provider, redactor)),
        }
    }
}

// the mock short circuit: LiteLLM answers from mock_response before it decodes the provider
fn remix_mock(
    call: &VideoRemixCall,
    redactor: &Redactor,
) -> Result<Option<VideoObject>, VideoCallFailure> {
    let argument_provider = call.custom_llm_provider.as_deref().unwrap_or_default();
    mock_answer(call.mock_response.as_ref(), "a video object")
        .map_err(|error| failure(VideoCallError::Mock(error), argument_provider, redactor))
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

#[path = "video_remix_call/tests/support.rs"]
pub(crate) mod support;

#[path = "video_remix_call/tests/remix_timeout.rs"]
mod remix_timeout;

#[path = "video_remix_call/tests/remix_provider.rs"]
mod remix_provider;

#[path = "video_remix_call/tests/mock_response.rs"]
mod mock_response;

#[path = "video_remix_call/tests/call_type.rs"]
mod call_type;

#[path = "video_remix_call/tests/async_wrapper.rs"]
mod async_wrapper;

#[path = "video_remix_call/tests/unsupported_provider.rs"]
mod unsupported_provider;

#[path = "video_remix_call/tests/extra_params.rs"]
mod extra_params;

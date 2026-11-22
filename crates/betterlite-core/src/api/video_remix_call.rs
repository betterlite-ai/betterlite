pub mod config;
pub mod error;
mod prepare;
pub mod shared;

use std::future::Future;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::config::{DEFAULT_PROVIDER, DEFAULT_TIMEOUT_SECONDS, VideoOperation, provider_config};
use self::error::{VideoCallError, VideoCallFailure, failure};
use self::prepare::{PreparedRemix, prepare_remix};
use self::shared::{VideoCallDeps, mock_answer};
use crate::api::video_records::VideoObject;
use crate::api::video_remix_list_transport::client::{VideoClient, VideoClientSource};
use crate::api::video_remix_list_transport::error::VideoRemixListTransportError;
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

// Everything one remix entry hands the transport: the pool, the collaborators, the call and its client
pub struct RemixRequest<'call, 'deps, 'env, Source: VideoClientSource> {
    pub transport: &'deps VideoRemixListTransport<Source>,
    pub deps: VideoCallDeps<'deps, 'env>,
    pub call: &'call VideoRemixCall,
    pub client: Option<VideoClient<'call, Source>>,
}

// ast-grep-ignore: no-one-line-functions - B3340 exposes LiteLLM's video_remix under this public name
pub fn video_remix<Source: VideoClientSource>(
    request: RemixRequest<'_, '_, '_, Source>,
) -> Result<VideoObject, VideoCallFailure> {
    remix_begin(request, false)?.finish()
}

// ast-grep-ignore: no-one-line-functions - B3340 exposes LiteLLM's avideo_remix under this public name
pub async fn avideo_remix<Source: VideoClientSource>(
    request: RemixRequest<'_, '_, '_, Source>,
) -> Result<VideoObject, VideoCallFailure> {
    remix_begin(request, true)?.finish_async().await
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

// What a remix entry has after the handler ran: its answer, the provider and the redactor an error prints with
struct RemixBegin<'redactor, Pending> {
    handled: Handled<VideoObject, Pending>,
    provider: String,
    redactor: &'redactor Redactor,
}

impl<'redactor, Pending> RemixBegin<'redactor, Pending>
where
    Pending: Future<Output = Result<VideoObject, VideoRemixListTransportError>>,
{
    // the sync entry's answer: the handler must have answered in place
    fn finish(self) -> Result<VideoObject, VideoCallFailure> {
        match self.handled {
            Handled::Done(object) => Ok(object),
            Handled::Pending(_pending) => Err(remix_pending(&self.provider, self.redactor)),
        }
    }

    // the async entry's answer: the handler's future, awaited and mapped like a direct failure
    async fn finish_async(self) -> Result<VideoObject, VideoCallFailure> {
        let pending = match self.handled {
            Handled::Pending(pending) => pending,
            Handled::Done(object) => return Ok(object),
        };
        pending
            .await
            .map_err(|error| failure(VideoCallError::from(error), &self.provider, self.redactor))
    }
}

// the mock short circuit, the provider, the config and the handler, which answers in place or with a future
fn remix_begin<'call, 'deps, 'env, Source: VideoClientSource>(
    request: RemixRequest<'call, 'deps, 'env, Source>,
    is_async: bool,
) -> Result<
    RemixBegin<
        'deps,
        impl Future<Output = Result<VideoObject, VideoRemixListTransportError>> + Send,
    >,
    VideoCallFailure,
> {
    let provider = remix_provider(request.call);
    let redactor = request.deps.redactor;
    let argument_provider = request
        .call
        .custom_llm_provider
        .as_deref()
        .unwrap_or_default();
    let mock = mock_answer(request.call.mock_response.as_ref(), "a video object")
        .map_err(|error| failure(VideoCallError::Mock(error), argument_provider, redactor))?;
    let handled = match mock {
        Some(object) => Handled::Done(object),
        None => {
            let VideoCallDeps {
                configs,
                logging,
                env,
                request_timeout_seconds,
                ..
            } = request.deps;
            let config = provider_config(configs, &provider, VideoOperation::Remix)?;
            let prepared =
                prepare_remix(logging, request.call, &provider, request_timeout_seconds)?;
            request
                .transport
                .remix(
                    config,
                    remix_wire_call(request.call, &provider, &prepared, request.client),
                    LogContext {
                        state: logging,
                        env,
                    },
                    is_async,
                )
                .map_err(|error| failure(error, &provider, redactor))?
        }
    };
    Ok(RemixBegin {
        handled,
        provider,
        redactor,
    })
}

// a sync handler that answered with a future where the entry expected a direct answer
fn remix_pending(provider: &str, redactor: &Redactor) -> VideoCallFailure {
    failure(
        VideoCallError::UnexpectedPending {
            operation: VideoOperation::Remix,
        },
        provider,
        redactor,
    )
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

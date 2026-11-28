use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use http::{HeaderMap as HttpHeaders, Method};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use url::Url;

use crate::api::responses_agentic_execution::LoggingView;
use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_compact_http::service::facts::Sent;
use crate::api::responses_compact_http::service::{
    ClientKind, ClientSource, SharedSession, SuppliedClient,
};
use crate::api::responses_hook_policy::callbacks::{
    CallbackLists, DeploymentHookDispatcher, NamedCallbackResolver, custom_logger_callbacks,
};
use crate::api::responses_stream_driver::source::PayloadSource;

use self::finish::{Finish, checked, transformed, typed};
use self::hook::Applied;
use self::request::wire;
use super::config::{CreateConfig, CreateParams, Headers, Params};
use super::error::CreateError;

mod finish;
mod hook;
mod proxied;
mod request;
mod source;

// The selected httpx client, whose sync and async forms differ only in the class the handler accepts
pub enum SentReply {
    Buffered(UpstreamResponse),
    Streaming(Box<dyn PayloadSource>),
}

pub trait CreateTransport: Send + Sync {
    fn send(
        &self,
        request: CreateRequest,
        stream: bool,
    ) -> impl Future<Output = Result<SentReply, TransportError>> + Send;
}

pub struct CreateDeps<'deps, C: CreateConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

// Both create handlers' inputs; fake_stream is the raw value so `is True` can be the identity check
pub struct CreateCall<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub optional_params: &'call Object,
    pub custom_llm_provider: &'call str,
    pub params: &'call CreateParams,
    pub extra_headers: Option<&'call Headers>,
    pub extra_body: Option<&'call Object>,
    pub timeout: Option<f64>,
    pub fake_stream: Option<&'call Json>,
    pub litellm_metadata: Option<&'call Params>,
    pub shared_session: Option<SharedSession>,
}

pub struct CreateHooks<'hooks, D: ?Sized, E: ?Sized> {
    pub callbacks: CallbackLists<'hooks>,
    pub resolver: &'hooks dyn NamedCallbackResolver,
    pub dispatcher: &'hooks D,
    pub executor: &'hooks E,
    pub logging: Option<LoggingView<'hooks>>,
}

// One POST the selected client sends, with the timeout the handler computed for it
pub struct CreateRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HttpHeaders,
    pub body: RequestBody,
    pub timeout: Duration,
}

// What the iterator B927 builds needs from the send, plus the context the handler preserved for it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamSetup {
    pub model: String,
    pub custom_llm_provider: String,
    pub litellm_metadata: Option<Params>,
    pub request_data: Option<Params>,
    pub call_type: Option<String>,
}

pub struct CreateLive {
    pub setup: StreamSetup,
    pub source: Box<dyn PayloadSource>,
}

pub struct CreateSynthetic {
    pub setup: StreamSetup,
    pub response: ResponsesApiResponse,
}

pub enum CreateOutcome {
    Value(Box<RawValue>),
    Live(Box<CreateLive>),
    Synthetic(Box<CreateSynthetic>),
}

pub(super) enum Selected<'deps, Client> {
    Supplied(&'deps Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    pub(super) const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

// The request as the transform, signing and pre-call steps leave it, plus what the iterator keeps
struct Staged {
    applied: Applied,
    mode: ClientKind,
    effective_stream: bool,
    stream_requested: bool,
    data: Object,
    context: Params,
    headers: Headers,
    api_base: String,
    params: Params,
    litellm_metadata: Option<Params>,
}

impl Staged {
    // A streaming send hands the selected client's source over; a buffered one is transformed and finished
    async fn respond<C, D, E>(
        &self,
        config: &C,
        hooks: &CreateHooks<'_, D, E>,
        answer: SentReply,
        log: &mut LogContext<'_, '_>,
    ) -> Result<CreateOutcome, CreateError>
    where
        C: CreateConfig,
        D: DeploymentHookDispatcher + ?Sized,
        E: ContinuationExecutor + ?Sized,
    {
        let setup = StreamSetup {
            model: self.applied.model.clone(),
            custom_llm_provider: self.applied.custom_llm_provider.clone(),
            litellm_metadata: self.litellm_metadata.clone(),
            request_data: Some(self.context.clone()),
            call_type: Some(String::from("responses")),
        };
        match answer {
            SentReply::Streaming(source) => {
                Ok(CreateOutcome::Live(Box::new(CreateLive { setup, source })))
            }
            SentReply::Buffered(buffered) => {
                let reply = transformed(config, &buffered, log)?;
                if self.stream_requested {
                    let response = typed(reply.response)?;
                    return Ok(CreateOutcome::Synthetic(Box::new(CreateSynthetic {
                        setup,
                        response,
                    })));
                }
                let callbacks = custom_logger_callbacks(&hooks.callbacks, hooks.resolver);
                let job = Finish {
                    applied: &self.applied,
                    mode: self.mode,
                    callbacks: &callbacks,
                    logging: hooks.logging,
                    executor: hooks.executor,
                    body: reply.response,
                };
                finish::finish(config, job, log).await
            }
        }
    }
}

// One shared sender for both handlers, following the source's statements in order
pub async fn run<C, S, D, E>(
    deps: &CreateDeps<'_, C, S>,
    mode: ClientKind,
    call: &CreateCall<'_>,
    hooks: &CreateHooks<'_, D, E>,
    log: &mut LogContext<'_, '_>,
) -> Result<CreateOutcome, CreateError>
where
    C: CreateConfig,
    S: ClientSource<Client: CreateTransport>,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
{
    let applied = match mode {
        ClientKind::Sync => {
            hook::apply(call, &hooks.callbacks, hooks.resolver, hooks.dispatcher).await?
        }
        ClientKind::Asynchronous => Applied::original(call),
    };
    let client = request::select_client(deps, mode, &applied.params, call.shared_session)?;
    let (headers, api_base) = request::validated_environment(deps.config, call, &applied)?;
    let mut data = request::transformed_data(deps.config, mode, call, &applied, &headers).await?;
    let (stream_requested, effective_stream) =
        request::stream_flags(call, &applied.optional_params, &mut data);
    let params = applied.params.to_map();
    let context = hook::request_context(&applied.input, &applied.optional_params, &params)?;
    let staged = Staged {
        applied,
        mode,
        effective_stream,
        stream_requested,
        data,
        context,
        headers,
        api_base,
        params,
        litellm_metadata: call.litellm_metadata.cloned(),
    };
    let (post, timeout) = wire(deps.config, mode, call, &staged, log).await?;
    let started = Instant::now();
    let reply = client.get().send(post, staged.effective_stream).await;
    let sent = Sent {
        mode,
        requested: Some(timeout),
        effective: timeout,
        elapsed: started.elapsed(),
    };
    let answer = checked(reply, staged.effective_stream, &sent, deps.config)?;
    staged.respond(deps.config, hooks, answer, log).await
}

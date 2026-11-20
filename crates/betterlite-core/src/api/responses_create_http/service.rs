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
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use http::{HeaderMap as HttpHeaders, Method};
use serde_json::value::RawValue;
use url::Url;

use crate::api::responses_agentic_execution::LoggingView;
use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_compact_http::body::signed_body_kwargs;
use crate::api::responses_compact_http::service::facts::Sent;
use crate::api::responses_compact_http::service::{ClientKind, SharedSession, SuppliedClient};
use crate::api::responses_hook_policy::callbacks::{
    CallbackLists, DeploymentHookDispatcher, NamedCallbackResolver, custom_logger_callbacks,
};
use crate::api::responses_stream_driver::source::PayloadSource;
use crate::api::vector_store_http_write::error::{ClientError, display_facts};

use self::finish::{checked, transformed, typed};
use self::hook::Applied;
use self::request::{endpoint_url, wire_body, wire_headers};
use super::config::{CreateConfig, CreateParams, CreateSignRequest, Headers, Params};
use super::convert::is_true;
use super::error::{CreateError, provider_error};

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

pub trait ClientSource: Send + Sync {
    type Client: CreateTransport;

    fn client(
        &self,
        ssl_verify: Option<bool>,
        shared_session: Option<SharedSession>,
    ) -> Result<Self::Client, ClientError>;
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

pub struct CreateHooks<'hooks, D: ?Sized> {
    pub callbacks: CallbackLists<'hooks>,
    pub resolver: &'hooks dyn NamedCallbackResolver,
    pub dispatcher: &'hooks D,
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
    Live(CreateLive),
    Synthetic(CreateSynthetic),
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

// One shared sender for both handlers, following the source's statements in order
pub async fn run<C, S, D, E>(
    deps: &CreateDeps<'_, C, S>,
    mode: ClientKind,
    call: &CreateCall<'_>,
    hooks: &CreateHooks<'_, D>,
    executor: &E,
    log: &mut LogContext<'_, '_>,
) -> Result<CreateOutcome, CreateError>
where
    C: CreateConfig,
    S: ClientSource,
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
    let requested_stream = applied
        .optional_params
        .get("stream")
        .is_some_and(PromptJson::is_truthy);
    let mut data = request::transform(deps.config, mode, &applied, &headers).await?;
    data = request::normalize(deps.config, data)?;
    if let Some(extra_body) = call.extra_body.filter(|body| !body.is_empty()) {
        let transformed = request::extra_body(deps.config, extra_body, &data, &applied)?;
        data.extend(transformed);
    }
    let is_stream_request =
        requested_stream || data.get("stream").is_some_and(PromptJson::is_truthy);
    let mut effective_stream = is_stream_request;
    let params = applied.params.to_map();
    let context = hook::request_context(&applied.input, &applied.optional_params, &params)?;
    if is_stream_request && is_true(call.fake_stream) {
        effective_stream = false;
        data.shift_remove("stream");
    }
    let signing = CreateSignRequest {
        headers: &headers,
        optional_params: &params,
        request_data: &data,
        api_base: &api_base,
        api_key: applied.params.api_key.as_ref(),
        model: &applied.model,
        stream: effective_stream,
        fake_stream: call.fake_stream,
    };
    let signed = request::sign(deps.config, mode, &signing).await?;
    let body = wire_body(&signed_body_kwargs(signed.body.as_deref(), &data))?;
    hook::pre_call(&applied.input, &api_base, &signed.headers, &data, log)?;
    let timeout = request::timeout(call.timeout, &applied.optional_params).map_err(|message| {
        CreateError::Provider(provider_error(&display_facts(message), deps.config))
    })?;
    let wire = CreateRequest {
        method: Method::POST,
        url: endpoint_url(&api_base)?,
        headers: wire_headers(&signed.headers)?,
        body,
        timeout,
    };
    let started = Instant::now();
    let reply = client.get().send(wire, effective_stream).await;
    let sent = Sent {
        mode,
        requested: Some(timeout),
        effective: timeout,
        elapsed: started.elapsed(),
    };
    let answer = checked(reply, effective_stream, &sent, deps.config)?;
    let setup = StreamSetup {
        model: applied.model.clone(),
        custom_llm_provider: applied.custom_llm_provider.clone(),
        litellm_metadata: call.litellm_metadata.cloned(),
        request_data: Some(context),
        call_type: Some(String::from("responses")),
    };
    match answer {
        SentReply::Streaming(source) => Ok(CreateOutcome::Live(CreateLive { setup, source })),
        SentReply::Buffered(answer) => {
            let reply = transformed(deps.config, answer, log)?;
            if is_stream_request {
                return Ok(CreateOutcome::Synthetic(CreateSynthetic {
                    setup,
                    response: typed(reply.response)?,
                }));
            }
            let callbacks = custom_logger_callbacks(&hooks.callbacks, hooks.resolver);
            finish::finish(
                deps.config,
                mode,
                &applied,
                &callbacks,
                hooks.logging,
                executor,
                reply.response,
                log,
            )
            .await
        }
    }
}

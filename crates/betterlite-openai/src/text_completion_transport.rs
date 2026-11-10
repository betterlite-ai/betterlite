pub mod call;
pub mod error;
pub mod logging;
pub mod reply;
pub mod sdk;
pub mod stream;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamOptions;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::text_completion_policy::prompt::PromptError;
use crate::text_completion_policy::{TextCompletionProviderTag, select_completion_provider};

use self::call::{
    CallContext, SentCompletion, TextCompletionCall, TextCompletionOutcome, TextCompletionReply,
    TextCompletionRequest, prepare,
};
use self::error::{ErrorMode, ErrorSource, TextCompletionError, TextCompletionFailure};
use self::logging::{CompletionLogger, PostCallRecord, ResponseKind};
use self::reply::text_response;
use self::sdk::{
    ClientConstructor, ClientKind, HttpClientSource, ParsedCompletion, SdkArguments, SdkError,
    TextCompletionSdk as _, missing_api_key,
};
use self::stream::{StreamSetup, completion_stream};

// LiteLLM's REPEATED_STREAMING_CHUNK_LIMIT default
const REPEATED_CHUNK_LIMIT: usize = 100;

pub type TransformFn =
    fn(&str, &[Value], Map<String, Value>) -> Result<Map<String, Value>, PromptError>;

// The provider configs installed by the provider families; B2237's selector picks the one a call uses
#[derive(Clone, Copy)]
pub struct TextCompletionConfigs {
    pub open_ai: TransformFn,
    pub fireworks_ai: TransformFn,
    pub together_ai: TransformFn,
    pub text_completion_inception: TransformFn,
}

impl TextCompletionConfigs {
    #[must_use]
    pub fn selected(&self, provider: ProviderId) -> TransformFn {
        match select_completion_provider(provider) {
            TextCompletionProviderTag::OpenAi => self.open_ai,
            TextCompletionProviderTag::FireworksAi => self.fireworks_ai,
            TextCompletionProviderTag::TogetherAi => self.together_ai,
            TextCompletionProviderTag::TextCompletionInception => self.text_completion_inception,
        }
    }
}

// Which SDK client a path builds and which of its errors the handler wraps
struct Route {
    kind: ClientKind,
    http_client: HttpClientSource,
    build: ErrorMode,
    send: ErrorMode,
}

const fn route(context: &CallContext) -> Route {
    match (context.acompletion, context.streaming) {
        (true, true) => Route {
            kind: ClientKind::Async,
            http_client: HttpClientSource::AClientSession,
            build: ErrorMode::Raw,
            send: ErrorMode::Raw,
        },
        (true, false) => Route {
            kind: ClientKind::Async,
            http_client: HttpClientSource::AsyncHttpClient,
            build: ErrorMode::Mapped,
            send: ErrorMode::Mapped,
        },
        (false, true) => Route {
            kind: ClientKind::Sync,
            http_client: HttpClientSource::ClientSession,
            build: ErrorMode::Raw,
            send: ErrorMode::Mapped,
        },
        (false, false) => Route {
            kind: ClientKind::Sync,
            http_client: HttpClientSource::ClientSession,
            build: ErrorMode::Mapped,
            send: ErrorMode::Mapped,
        },
    }
}

fn failure(mode: ErrorMode, source: SdkError) -> TextCompletionFailure {
    match mode {
        ErrorMode::Mapped => TextCompletionFailure::Wrapped(TextCompletionError::wrapped(
            ErrorSource::Sdk(Box::new(source)),
        )),
        ErrorMode::Raw => TextCompletionFailure::Raw(source),
    }
}

pub struct TextCompletionTransport<Constructor> {
    pub constructor: Constructor,
    pub configs: TextCompletionConfigs,
    pub now: fn() -> StreamInstant,
    pub next_id: fn() -> Uuid,
}

impl<Constructor: ClientConstructor> TextCompletionTransport<Constructor>
where
    Constructor::Client: Clone,
{
    pub fn begin(
        &self,
        request: TextCompletionRequest,
        provider: ProviderId,
        logger: &mut dyn CompletionLogger,
    ) -> Result<TextCompletionCall, TextCompletionFailure> {
        Ok(TextCompletionCall {
            context: prepare(&self.configs, request, provider, logger)?,
        })
    }

    pub async fn send(
        &self,
        call: &TextCompletionCall,
        supplied: Option<&Constructor::Client>,
    ) -> Result<SentCompletion, TextCompletionFailure> {
        let context = &call.context;
        let route = route(context);
        let client = if let Some(client) = supplied {
            client.clone()
        } else {
            let arguments = SdkArguments {
                api_key: context.api_key.clone(),
                base_url: context.api_base.clone(),
                timeout: context.timeout,
                max_retries: context.max_retries,
                organization: context.organization.clone(),
                http_client: route.http_client,
            };
            self.constructor
                .construct(&arguments, route.kind)
                .and_then(|built| built.ok_or_else(missing_api_key))
                .map_err(|source| failure(route.build, source))?
        };
        let raw = client
            .create(&context.data)
            .await
            .map_err(|source| failure(route.send, source))?;
        match raw.parse().map_err(|source| failure(route.send, source))? {
            ParsedCompletion::Buffered(reply) => Ok(SentCompletion::Buffered(
                serde_json::to_value(reply.as_ref())?,
            )),
            ParsedCompletion::Stream(bytes) => Ok(SentCompletion::Stream(bytes)),
        }
    }

    pub fn finish(
        &self,
        call: &TextCompletionCall,
        sent: SentCompletion,
        logger: &mut dyn CompletionLogger,
    ) -> Result<TextCompletionOutcome, TextCompletionFailure> {
        let context = &call.context;
        match sent {
            SentCompletion::Buffered(dumped) => {
                record_buffered_reply(context, logger, dumped).map(TextCompletionOutcome::Buffered)
            }
            SentCompletion::Stream(bytes) => {
                let setup = stream_setup(context, logger, self.now, self.next_id);
                Ok(TextCompletionOutcome::Stream(completion_stream(
                    bytes, setup,
                )))
            }
        }
    }
}

fn record_buffered_reply(
    context: &CallContext,
    logger: &mut dyn CompletionLogger,
    dumped: Value,
) -> Result<TextCompletionReply, TextCompletionFailure> {
    let kind = if context.acompletion {
        ResponseKind::SdkObject
    } else {
        ResponseKind::DumpedDict
    };
    logger.post_call(PostCallRecord {
        api_key: context.api_key.clone(),
        original_response: dumped.clone(),
        kind,
        headers: context.headers.clone(),
        api_base: context.api_base.clone(),
    });
    let hidden = if context.acompletion {
        Some(serde_json::to_string(&dumped)?)
    } else {
        None
    };
    Ok(TextCompletionReply {
        response: text_response(dumped),
        hidden_original_response: hidden,
    })
}

fn stream_setup(
    context: &CallContext,
    logger: &dyn CompletionLogger,
    now: fn() -> StreamInstant,
    next_id: fn() -> Uuid,
) -> StreamSetup {
    let defaults = logger.stream_defaults();
    let stream_options = match context.data.get("stream_options") {
        Some(Value::Object(fields)) => Some(StreamOptions {
            include_usage: matches!(fields.get("include_usage"), Some(Value::Bool(true))),
        }),
        _ => defaults.stream_options,
    };
    StreamSetup {
        model: context.model.clone(),
        stream_options,
        merge_reasoning_in_choices: defaults.merge_reasoning_in_choices,
        is_function_call: defaults.is_function_call,
        base_hidden_params: defaults.base_hidden_params,
        repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
        now,
        next_id,
    }
}

mod tests;

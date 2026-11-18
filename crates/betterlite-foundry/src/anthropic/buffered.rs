use std::error::Error as StdError;

use betterlite_anthropic::anthropic_http_stream::error::PostFailure;
use betterlite_anthropic::anthropic_http_stream::transport::{StreamPost, StreamTransport};
use betterlite_anthropic::chat_call::ChatDeps;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::chat_call::config::{ChatConfig, TransformResponse};
use betterlite_anthropic::chat_call::error::ChatCallError;
use betterlite_anthropic::chat_call::transport::{BufferedReply, ChatTransport, ClientKind};
use betterlite_anthropic::response::{Logging, RawResponse, ResponseRequest};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use futures_util::future::LocalBoxFuture;
use indexmap::IndexMap;
use url::Url;

use super::error::ChatError;
use super::handler::CompletionResult;
use super::request::{ChatRequest, Prepared, Style};

struct Facts<'call> {
    request: ChatRequest<'call>,
    prepared: Prepared,
}

pub(super) fn send<'call, Config, Client, Counter>(
    request: ChatRequest<'call>,
    prepared: Prepared,
    deps: ChatDeps<'call, Config, Client, Counter>,
) -> LocalBoxFuture<'call, CompletionResult<Client, Counter>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + 'call,
{
    Box::pin(async move {
        let mut deps = deps;
        let transport = transport_of(&deps, request.style);
        let reply = transport
            .post_buffered(StreamPost {
                url: Url::parse(request.api_base)?,
                headers: prepared.header_map,
                body: python_json::dumps_object(&prepared.body).into_bytes(),
                stream: false,
                deadline: request.timeout,
            })
            .await;
        let logging = Logging {
            state: &mut *deps.state,
            env: deps.env,
        };
        let facts = Facts { request, prepared };
        match reply {
            Ok(answer) => reply_outcome(deps.config, facts, answer, logging, &deps.counter),
            Err(failure) => Err(failure_of(
                failure,
                &facts.request,
                &facts.prepared.body,
                logging,
            )),
        }
    })
}

// async takes any supplied client; sync keeps only an HTTPHandler and otherwise opens its own
fn transport_of<'clients, Config, Client, Counter>(
    deps: &'clients ChatDeps<'_, Config, Client, Counter>,
    style: Style,
) -> &'clients Client
where
    Client: StreamTransport,
{
    match deps.clients.supplied.as_ref() {
        Some(supplied) if style == Style::Async || supplied.kind == ClientKind::Sync => {
            supplied.transport
        }
        _ => deps.clients.pooled,
    }
}

fn reply_outcome<Config, ClientFailure, Counter>(
    config: &Config,
    facts: Facts<'_>,
    reply: BufferedReply,
    logging: Logging<'_>,
    counter: &Counter,
) -> Result<ChatOutcome<ClientFailure, Counter>, ChatError<ClientFailure, Counter::Failure>>
where
    Config: ChatConfig,
    ClientFailure: StdError + Send + Sync + 'static,
    Counter: ReasoningTokenCounter,
{
    let BufferedReply {
        status_code,
        headers,
        body,
    } = reply;
    let response = ResponseRequest {
        messages: &facts.request.messages,
        optional_params: &facts.prepared.optional_params,
        litellm_params: &facts.request.litellm_params,
        request_data: &facts.prepared.body,
        json_mode: Some(facts.prepared.json_mode),
        now: facts.request.now,
        disable_add_prefix_to_prompt: facts.request.disable_add_prefix_to_prompt,
    };
    let mut envelope = facts.request.model_response;
    config.transform_response(
        TransformResponse {
            envelope: &mut envelope,
            raw: &RawResponse {
                status: status_code,
                headers: &headers,
                body: &body,
            },
            request: &response,
        },
        logging,
        counter,
    )?;
    Ok(ChatOutcome::Reply(Box::new(envelope)))
}

// Both buffered failures raise AnthropicError; only the async path logs the exception before wrapping it
fn failure_of<ClientFailure, CounterFailure>(
    failure: PostFailure<ClientFailure>,
    request: &ChatRequest<'_>,
    body: &IndexMap<String, Json>,
    logging: Logging<'_>,
) -> ChatError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    let Logging { state, env } = logging;
    let (status_code, message, headers) = match failure {
        PostFailure::Status(status) => {
            let details = *status;
            (
                details.status_code,
                body_text(&details.body),
                details.exception_headers.or(details.response_headers),
            )
        }
        // getattr(e, "status_code", 500): a plain transport failure, a timeout included, carries no status
        PostFailure::Known(source) | PostFailure::Unknown(source) => {
            (500, source.to_string(), None)
        }
    };
    if request.style == Style::Async {
        post_call(
            state,
            &PostCall {
                input: Json::List(request.messages.clone()),
                original_response: Json::Text(message.clone()),
                additional_args: AdditionalArgs {
                    api_base: None,
                    headers: None,
                    complete_input_dict: Some(RawBody::Object(body.clone())),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            env,
        );
    }
    ChatError::failure(status_code, message, headers)
}

// httpx's errors="replace": each invalid sequence becomes one U+FFFD in the message the handler raises
fn body_text(body: &[u8]) -> String {
    let mut text = String::with_capacity(body.len());
    for chunk in body.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{FFFD}');
        }
    }
    text
}

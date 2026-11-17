use std::error::Error as StdError;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use url::Url;

use crate::anthropic_http_stream::call::StreamBuild;
use crate::anthropic_http_stream::error::PostFailure;
use crate::anthropic_http_stream::transport::StreamPost;
use crate::response::{Logging, RawResponse, ResponseRequest};
use crate::usage::ReasoningTokenCounter;

use super::config::ChatConfig;
use super::error::ChatCallError;
use super::prepare::{Options, Prepared};
use super::transport::{ChatTransport, ClientKind};
use super::{ChatCall, ChatClients, ChatDeps, ChatOutcome};

mod streams;

use streams::{StreamFacts, open_async, open_sync};

struct BufferedCall<'call> {
    call: ChatCall<'call>,
    body: IndexMap<String, Json>,
    header_map: http::HeaderMap,
    optional_params: IndexMap<String, Json>,
    options: Options,
}

pub(super) async fn send<Config, Client, Counter>(
    call: ChatCall<'_>,
    deps: ChatDeps<'_, Config, Client, Counter>,
    prepared: Prepared,
) -> Result<ChatOutcome<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Clone + Unpin + Send,
{
    let Prepared {
        body,
        header_map,
        optional_params,
        options,
    } = prepared;
    let ChatDeps {
        config,
        clients,
        state,
        env,
        stream_logger,
        counter,
        ids,
        max_event_bytes,
        max_error_body_bytes,
    } = deps;
    if options.stream {
        // the stream helper takes the messages as one JSON list, the way LiteLLM passes its copied list
        let messages = Json::List(call.messages);
        let facts = StreamFacts {
            model: call.model,
            api_base: call.api_base,
            messages: &messages,
            data: body,
            headers: header_map,
            timeout: call.timeout,
            json_mode: options.json_mode,
            optional_params: &optional_params,
            litellm_params: &call.litellm_params,
        };
        let build = StreamBuild {
            ids,
            counter: counter.clone(),
            max_event_bytes,
            max_error_body_bytes,
        };
        return if call.acompletion {
            open_async(clients, stream_logger, build, facts).await
        } else {
            open_sync(clients, stream_logger, build, facts).await
        };
    }
    buffered(
        BufferedCall {
            call,
            body,
            header_map,
            optional_params,
            options,
        },
        config,
        &clients,
        state,
        env,
        &counter,
    )
    .await
}

async fn buffered<Config, Client, Counter>(
    facts: BufferedCall<'_>,
    config: &Config,
    clients: &ChatClients<'_, Client>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    counter: &Counter,
) -> Result<ChatOutcome<Client::Failure, Counter>, ChatCallError<Client::Failure, Counter::Failure>>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter,
{
    let BufferedCall {
        call,
        body,
        header_map,
        optional_params,
        options,
    } = facts;
    let client = match clients.supplied.as_ref() {
        // async takes any supplied client; sync keeps only an HTTPHandler and otherwise opens its own
        Some(supplied) if call.acompletion || supplied.kind == ClientKind::Sync => {
            supplied.transport
        }
        _ => clients.pooled,
    };
    let url = Url::parse(call.api_base).map_err(|source| ChatCallError::Endpoint { source })?;
    let body_text = python_json::dumps_object(&body);
    let reply = client
        .post_buffered(StreamPost {
            url,
            headers: header_map,
            body: body_text.into_bytes(),
            stream: false,
            deadline: call.timeout,
        })
        .await;
    let reply = match reply {
        Ok(reply) => reply,
        Err(failure) => return Err(buffered_failure(failure, &call, &body, state, env)),
    };
    let raw = RawResponse {
        status: reply.status_code,
        headers: &reply.headers,
        body: &reply.body,
    };
    let request = ResponseRequest {
        messages: &call.messages,
        optional_params: &optional_params,
        litellm_params: &call.litellm_params,
        request_data: &body,
        json_mode: Some(options.json_mode),
        now: call.now,
        disable_add_prefix_to_prompt: call.disable_add_prefix_to_prompt,
    };
    let mut envelope = call.model_response;
    config.transform_response(
        &mut envelope,
        &raw,
        &request,
        Logging { state, env },
        counter,
    )?;
    Ok(ChatOutcome::Reply(Box::new(envelope)))
}

// Both buffered paths raise the transport failure as AnthropicError; only the async path logs it first
fn buffered_failure<ClientFailure, CounterFailure>(
    failure: PostFailure<ClientFailure>,
    call: &ChatCall<'_>,
    body: &IndexMap<String, Json>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> ChatCallError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    let (status_code, message, headers) = match failure {
        PostFailure::Status(status) => {
            let status = *status;
            (
                status.status_code,
                text_of(status.body),
                status.exception_headers.or(status.response_headers),
            )
        }
        // getattr(e, "status_code", 500): a plain transport failure, a timeout included, carries no status
        PostFailure::Known(source) | PostFailure::Unknown(source) => {
            (500, source.to_string(), None)
        }
    };
    if call.acompletion {
        post_call(
            state,
            &PostCall {
                input: Json::List(call.messages.clone()),
                original_response: Json::Text(message.clone()),
                additional_args: AdditionalArgs {
                    complete_input_dict: Some(RawBody::Object(body.clone())),
                    ..AdditionalArgs::default()
                },
            },
            env,
        );
    }
    ChatCallError::Anthropic {
        status_code,
        message,
        headers,
    }
}

// httpx decodes the body with replacement characters; a body that is not text at all still needs a message
fn text_of(body: Vec<u8>) -> String {
    String::from_utf8(body)
        .unwrap_or_else(|error| format!("{} bytes that are not text", error.as_bytes().len()))
}

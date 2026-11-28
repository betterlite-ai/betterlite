use std::error::Error as StdError;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use url::Url;

use crate::anthropic_http_stream::error::PostFailure;
use crate::anthropic_http_stream::transport::StreamPost;
use crate::response::{Logging, RawResponse, ResponseRequest};
use crate::usage::ReasoningTokenCounter;

use super::config::{ChatConfig, TransformResponse};
use super::error::ChatCallError;
use super::prepare::Prepared;
use super::transport::{BufferedReply, ChatTransport};
use super::{CallFacts, CallStyle, ChatDeps, ChatOutcome, ChatResult};
use betterlite_http::client_kind::ClientKind;

pub mod streams;
struct ReplyFacts<'call> {
    call: CallFacts<'call>,
    body: IndexMap<String, Json>,
    optional_params: IndexMap<String, Json>,
    json_mode: bool,
}
pub async fn buffered<'call, Config, Client, Counter>(
    call: CallFacts<'call>,
    prepared: Prepared,
    deps: ChatDeps<'call, Config, Client, Counter>,
) -> ChatResult<Client, Counter>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Send + 'call,
{
    let counter = deps.counter;
    // async takes any supplied client; sync keeps only an HTTPHandler and otherwise opens its own
    let client = match deps.clients.supplied.as_ref() {
        Some(supplied) if call.style == CallStyle::Async || supplied.kind == ClientKind::Sync => {
            supplied.transport
        }
        _ => deps.clients.pooled,
    };
    let reply = client
        .post_buffered(StreamPost {
            url: Url::parse(call.api_base)?,
            headers: prepared.header_map,
            body: python_json::dumps_object(&prepared.body).into_bytes(),
            stream: false,
            deadline: call.timeout,
        })
        .await;
    let logging = Logging {
        state: &mut *deps.state,
        env: deps.env,
    };
    let facts = ReplyFacts {
        call,
        body: prepared.body,
        optional_params: prepared.optional_params,
        json_mode: prepared.json_mode,
    };
    match reply {
        Ok(answer) => reply_outcome(deps.config, facts, answer, logging, &counter),
        Err(failure) => Err(buffered_failure(failure, &facts.call, &facts.body, logging)),
    }
}
fn reply_outcome<Config, ClientFailure, Counter>(
    config: &Config,
    facts: ReplyFacts<'_>,
    reply: BufferedReply,
    logging: Logging<'_>,
    counter: &Counter,
) -> Result<ChatOutcome<ClientFailure, Counter>, ChatCallError<ClientFailure, Counter::Failure>>
where
    Config: ChatConfig,
    ClientFailure: StdError + Send + Sync + 'static,
    Counter: ReasoningTokenCounter,
{
    let BufferedReply {
        status_code,
        headers,
        body: reply_body,
    } = reply;
    let request = ResponseRequest {
        messages: &facts.call.messages,
        optional_params: &facts.optional_params,
        litellm_params: &facts.call.litellm_params,
        request_data: &facts.body,
        json_mode: Some(facts.json_mode),
        now: facts.call.now,
        disable_add_prefix_to_prompt: facts.call.disable_add_prefix_to_prompt,
    };
    let mut envelope = facts.call.model_response;
    config.transform_response(
        TransformResponse {
            envelope: &mut envelope,
            raw: &RawResponse {
                status: status_code,
                headers: &headers,
                body: &reply_body,
            },
            request: &request,
        },
        logging,
        counter,
    )?;
    Ok(ChatOutcome::Reply(Box::new(envelope)))
}
// Both buffered paths raise the transport failure as AnthropicError; only the async path logs it first
fn buffered_failure<ClientFailure, CounterFailure>(
    failure: PostFailure<ClientFailure>,
    call: &CallFacts<'_>,
    body: &IndexMap<String, Json>,
    logging: Logging<'_>,
) -> ChatCallError<ClientFailure, CounterFailure>
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
                text_of(details.body),
                details.exception_headers.or(details.response_headers),
            )
        }
        // getattr(e, "status_code", 500): a plain transport failure, a timeout included, carries no status
        PostFailure::Known(source) | PostFailure::Unknown(source) => {
            (500, source.to_string(), None)
        }
    };
    if call.style == CallStyle::Async {
        post_call(
            state,
            &PostCall {
                input: Json::List(call.messages.clone()),
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

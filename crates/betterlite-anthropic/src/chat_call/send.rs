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
use super::prepare::Options;
use super::transport::{BufferedReply, ChatTransport};
use super::{ChatCall, ChatDeps, ChatOutcome};

pub(super) mod streams;

pub(super) struct BufferedCall<'call> {
    pub(super) call: ChatCall<'call>,
    pub(super) body: IndexMap<String, Json>,
    pub(super) header_map: http::HeaderMap,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) options: Options,
}

// The buffered branch: one POST with the prepared body, then the selected config fills in the reply
pub(super) async fn buffered<Config, Client, Counter>(
    facts: BufferedCall<'_>,
    client: &Client,
    config: &Config,
    logging: Logging<'_>,
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
    let url = Url::parse(call.api_base)?;
    let reply = client
        .post_buffered(StreamPost {
            url,
            headers: header_map,
            body: python_json::dumps_object(&body).into_bytes(),
            stream: false,
            deadline: call.timeout,
        })
        .await;
    let reply = match reply {
        Ok(reply) => reply,
        Err(failure) => return Err(buffered_failure(failure, &call, &body, logging)),
    };
    let request = response_request(&call, &body, &optional_params, options.json_mode);
    let mut envelope = call.model_response;
    config.transform_response(
        TransformResponse {
            envelope: &mut envelope,
            raw: &RawResponse {
                status: reply.status_code,
                headers: &reply.headers,
                body: &reply.body,
            },
            request: &request,
        },
        logging,
        counter,
    )?;
    Ok(ChatOutcome::Reply(Box::new(envelope)))
}

// the arguments LiteLLM's transform_response reads back off the caller's own request
fn response_request<'request>(
    call: &'request ChatCall<'_>,
    body: &'request IndexMap<String, Json>,
    optional_params: &'request IndexMap<String, Json>,
    json_mode: bool,
) -> ResponseRequest<'request> {
    ResponseRequest {
        messages: &call.messages,
        optional_params,
        litellm_params: &call.litellm_params,
        request_data: body,
        json_mode: Some(json_mode),
        now: call.now,
        disable_add_prefix_to_prompt: call.disable_add_prefix_to_prompt,
    }
}

// Both buffered paths raise the transport failure as AnthropicError; only the async path logs it first
fn buffered_failure<ClientFailure, CounterFailure>(
    failure: PostFailure<ClientFailure>,
    call: &ChatCall<'_>,
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

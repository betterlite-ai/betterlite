use betterlite_anthropic::stream_state::state::IdSource;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_response_stream::source::ResponsesEventSource;
use crate::api::messages_response_stream::{MessagesResponseStream, MessagesResponseStreamInput};
use crate::api::messages_responses_output::records::ResponseRecord;
use crate::api::messages_responses_output::response::translate_response;
use crate::api::messages_thinking::bridge::local_model_name;
use crate::api::response_id_traversal::response_id::transcode;
use crate::api::responses_create::error::CreateError;
use crate::api::responses_create_http::service::LogContext;

use super::error::MessagesResponseCallError;
use super::forwarding::forwarded_provider;
use super::records::{
    MessagesResponseCallAnswer, MessagesResponseCallInput, MessagesResponseCallResult,
    ResponsesAnswer,
};
use super::request::build_responses_call;
use super::seams::{MessagesResponseCallSeams, ResponsesCreatePort};

// LiteLLM keeps call_type at anthropic_messages so the spend row bills /v1/messages, not /v1/responses.
pub(crate) const ANTHROPIC_MESSAGES_CALL_TYPE: &str = "anthropic_messages";

// The handler both sync and async entries share: build the kwargs, call litellm.aresponses, then wrap the stream or translate the response.
pub async fn messages_response_call<S, P>(
    input: &MessagesResponseCallInput,
    seams: &MessagesResponseCallSeams<'_>,
    port: &P,
    ids: &dyn IdSource,
    log: &mut LogContext<'_, '_>,
) -> Result<MessagesResponseCallResult<S>, MessagesResponseCallError>
where
    S: ResponsesEventSource,
    P: ResponsesCreatePort<Stream = S>,
{
    let call = build_responses_call(input, seams)?;
    log.state.call_type = String::from(ANTHROPIC_MESSAGES_CALL_TYPE);
    let upstream = port
        .create(&call, log)
        .await
        .map_err(MessagesResponseCallError::Create)?;
    let answer = if input.stream {
        stream_answer(upstream, input, seams, ids, log)?
    } else {
        native_answer(upstream, &seams.limits)?
    };
    Ok(MessagesResponseCallResult { call, answer })
}

// if stream: AnthropicResponsesStreamWrapper(responses_stream=result, model=local_model_name(model, provider_hint)).
fn stream_answer<S>(
    answer: ResponsesAnswer<S>,
    input: &MessagesResponseCallInput,
    seams: &MessagesResponseCallSeams<'_>,
    ids: &dyn IdSource,
    log: &mut LogContext<'_, '_>,
) -> Result<MessagesResponseCallAnswer<S>, MessagesResponseCallError>
where
    S: ResponsesEventSource,
{
    let stream = match answer {
        ResponsesAnswer::Stream(stream) => stream,
        ResponsesAnswer::Response(_) => {
            return Err(MessagesResponseCallError::Unexpected {
                kind: String::from("a ResponsesAPIResponse"),
            });
        }
        ResponsesAnswer::Other { kind } => {
            return Err(MessagesResponseCallError::Unexpected { kind });
        }
    };
    let provider_hint = forwarded_provider(&input.forwarded)?;
    let model = local_model_name(&input.request.model, provider_hint);
    let wrapper = MessagesResponseStream::new(
        stream,
        ids,
        Some(&mut *log.state),
        MessagesResponseStreamInput {
            model: model.to_owned(),
            redactor: seams.redactor.clone(),
            limits: seams.limits,
        },
    );
    Ok(MessagesResponseCallAnswer::Stream(Box::new(wrapper)))
}

// if not isinstance(result, ResponsesAPIResponse): raise ValueError(f"Expected ResponsesAPIResponse, got {type(result)}").
fn native_answer<S>(
    answer: ResponsesAnswer<S>,
    limits: &ApiLimits,
) -> Result<MessagesResponseCallAnswer<S>, MessagesResponseCallError> {
    match answer {
        ResponsesAnswer::Response(response) => {
            let record: ResponseRecord = transcode(response.as_ref()).map_err(CreateError::from)?;
            let message = translate_response(&record, limits)?;
            Ok(MessagesResponseCallAnswer::Native(Box::new(message)))
        }
        ResponsesAnswer::Stream(_) => Err(MessagesResponseCallError::Unexpected {
            kind: String::from("a stream"),
        }),
        ResponsesAnswer::Other { kind } => Err(MessagesResponseCallError::Unexpected { kind }),
    }
}

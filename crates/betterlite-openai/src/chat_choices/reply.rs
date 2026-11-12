use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;

use crate::compatible_wire::LogInput;
use crate::openai_base_config::Headers;
use crate::openai_base_config::forwarding::{
    BaseConfigError, ForwardingFailure, OpenAiErrorParts, OpenAiErrorRecord, ResponseForward,
    forward_response,
};

#[derive(Debug, Error)]
pub enum TransformResponseError {
    #[error("the provider answer is not valid JSON")]
    Json(Box<OpenAiErrorRecord>),
    #[error(transparent)]
    Projection(#[from] ForwardingFailure),
    #[error("api_base is not a valid URL")]
    Base {
        #[source]
        source: url::ParseError,
    },
}

pub struct ResponseReply<'response> {
    pub status: u16,
    pub headers: &'response Headers,
    pub body: &'response [u8],
}

// OpenAIGPTConfig.transform_response: the post-call log sees the raw text, then the reply is decoded
pub fn transform_response<Projection>(
    reply: &ResponseReply<'_>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    input: LogInput,
    projection: Projection,
) -> Result<Value, TransformResponseError>
where
    Projection: FnOnce(Value, &Headers, &Headers) -> Result<Value, ForwardingFailure>,
{
    #[expect(
        clippy::disallowed_methods,
        reason = "LiteLLM reads raw_response.text leniently, logs it, then fails in .json()"
    )]
    let text = String::from_utf8_lossy(reply.body).into_owned();
    let call = PostCall {
        original_response: Json::Text(text.clone()),
        input: input.messages,
        additional_args: AdditionalArgs {
            api_base: None,
            headers: None,
            complete_input_dict: Some(input.data),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    let wire = ResponseForward {
        raw_text: &text,
        raw_headers: reply.headers,
        hidden_headers: reply.headers,
    };
    forward_response(
        |_raw_text, _headers| {
            post_call(state, &call, env);
            Ok(())
        },
        projection,
        &wire,
    )
    .map_err(|error| failure(error, reply))
}

fn failure(error: BaseConfigError, reply: &ResponseReply<'_>) -> TransformResponseError {
    match error {
        BaseConfigError::Decode(_) => TransformResponseError::Json(Box::new(
            OpenAiErrorRecord::from_parts(OpenAiErrorParts {
                status_code: reply.status,
                message: String::from("the provider answer is not valid JSON"),
                headers: Some(reply.headers.clone()),
                request: None,
                response: None,
                body: None,
            }),
        )),
        BaseConfigError::Projection(failure) | BaseConfigError::Sink(failure) => {
            TransformResponseError::Projection(failure)
        }
        BaseConfigError::InvalidUrl(source) => TransformResponseError::Base { source },
    }
}

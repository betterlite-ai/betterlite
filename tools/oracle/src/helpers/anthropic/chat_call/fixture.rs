use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::anthropic_http_stream::error::{PostFailure, StatusFailure};
use betterlite_anthropic::anthropic_http_stream::transport::{
    LineChunks, StreamPost, StreamReply, StreamTransport,
};
use betterlite_anthropic::chat_call::config::{ChatConfig, TransformRequest, TransformResponse};
use betterlite_anthropic::chat_call::error::ConfigError;
use betterlite_anthropic::chat_call::transport::{BufferedReply, ChatTransport};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::request::beta_headers::add_optional_betas;
use betterlite_anthropic::response::error::ResponseError;
use betterlite_anthropic::response::{Logging, transform_response};
use betterlite_anthropic::stream_decode::TransportChunk;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_core::api::anthropic_beta_config::matrix::{BetaMatrix, BetaNode};
use betterlite_core::api::anthropic_beta_filter::matrix_filter::filter_request_betas;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Json as PromptJson;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;

use super::Input;

pub(crate) mod runtime;

#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct FixtureFailure {
    pub(crate) message: String,
}

pub(crate) enum Trip {
    Reply,
    Status {
        status_code: u16,
        headers: HeaderMap,
        body: Vec<u8>,
    },
    Unknown(String),
}

pub(crate) struct FixtureReply {
    pub(crate) status_code: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
    pub(crate) lines: Vec<String>,
}

pub(crate) struct FixtureTransport {
    reply: FixtureReply,
    trip: Trip,
    pub(crate) posts: Mutex<Vec<StreamPost>>,
}

impl FixtureTransport {
    pub(crate) const fn new(reply: FixtureReply, trip: Trip) -> Self {
        Self {
            reply,
            trip,
            posts: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, post: StreamPost) {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(post);
    }

    fn failure(&self) -> Option<PostFailure<FixtureFailure>> {
        match &self.trip {
            Trip::Reply => None,
            Trip::Status {
                status_code,
                headers,
                body,
            } => Some(PostFailure::Status(Box::new(StatusFailure {
                status_code: *status_code,
                body: body.clone(),
                // httpx masks a status error, so the handler reads the headers off the response
                exception_headers: None,
                response_headers: Some(headers.clone()),
            }))),
            Trip::Unknown(message) => Some(PostFailure::Unknown(FixtureFailure {
                message: message.clone(),
            })),
        }
    }
}

impl StreamTransport for FixtureTransport {
    type Failure = FixtureFailure;

    fn post(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<StreamReply<FixtureFailure>, PostFailure<FixtureFailure>>> + Send
    {
        self.record(post);
        if let Some(failure) = self.failure() {
            return ready(Err(failure));
        }
        let lines: Vec<String> = self.reply.lines.clone();
        let decoded: LineChunks<FixtureFailure> =
            stream::iter(lines.into_iter().map(|line| Ok(TransportChunk::Text(line)))).boxed();
        ready(Ok(StreamReply {
            status_code: self.reply.status_code,
            headers: self.reply.headers.clone(),
            lines: decoded,
        }))
    }
}

impl ChatTransport for FixtureTransport {
    fn post_buffered(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<BufferedReply, PostFailure<FixtureFailure>>> + Send {
        self.record(post);
        ready(self.failure().map_or_else(
            || {
                Ok(BufferedReply {
                    status_code: self.reply.status_code,
                    headers: self.reply.headers.clone(),
                    body: self.reply.body.clone(),
                })
            },
            Err,
        ))
    }
}

pub(crate) struct FixtureConfig {
    body: IndexMap<String, Json>,
    failure: Option<String>,
    matrix: BetaMatrix,
    pre_call: Mutex<Option<IndexMap<String, Json>>>,
}

impl FixtureConfig {
    pub(crate) fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        let BetaNode::Map(entries) = &input.beta_matrix else {
            return Err("the beta headers matrix is not an object".to_owned()).or_value_error();
        };
        Ok(Self {
            body: input.transform_body.clone(),
            failure: input.transform_failure.clone(),
            matrix: BetaMatrix(entries.clone()),
            pre_call: Mutex::new(None),
        })
    }

    // the crate's mapper logs post_call over this record, so the fixture keeps pre_call's own copy first
    pub(crate) fn pre_call_record(&self) -> Option<IndexMap<String, Json>> {
        self.pre_call
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn transform(&self) -> Result<IndexMap<String, Json>, ConfigError> {
        self.failure.as_ref().map_or_else(
            || Ok(self.body.clone()),
            |message| {
                Err(Box::new(FixtureFailure {
                    message: message.clone(),
                }))
            },
        )
    }
}

// the beta callbacks read the message and tool lists off the body the transform produced
fn prompt_list<'prompt>(
    prompt: &'prompt IndexMap<String, PromptJson>,
    name: &str,
) -> &'prompt [PromptJson] {
    match prompt.get(name) {
        Some(PromptJson::List(items)) => items.as_slice(),
        None
        | Some(
            PromptJson::Null
            | PromptJson::Bool(_)
            | PromptJson::Number(_)
            | PromptJson::Text(_)
            | PromptJson::Object(_),
        ) => &[],
    }
}
impl ChatConfig for FixtureConfig {
    fn transform_request(
        &self,
        _request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        self.transform()
    }

    fn async_transform_request(
        &self,
        _request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send {
        ready(self.transform())
    }

    // the beta dependency sits below the provider leaf, so this adapter owns the two-way JSON conversion
    fn filter_request_betas(
        &self,
        provider: &str,
        headers: &mut IndexMap<String, String>,
        body: &mut IndexMap<String, Json>,
    ) -> Result<(), ConfigError> {
        // a text round trip keeps the key order Python's dicts have; a serde_json::Value one sorts it
        let encoded = serde_json::to_string(&*body)?;
        let mut prompt: IndexMap<String, PromptJson> = serde_json::from_str(&encoded)?;
        // the provider config this fixture stands in for adds its own betas before the matrix filters them
        let messages = prompt_list(&prompt, "messages");
        let tools = prompt_list(&prompt, "tools");
        let mut native = AnthropicHeaders {
            values: headers.clone(),
        };
        add_optional_betas(&mut native, body, tools, messages, provider)?;
        *headers = native.values;
        filter_request_betas(headers, &mut prompt, &self.matrix, provider)?;
        let filtered = serde_json::to_string(&prompt)?;
        *body = serde_json::from_str(&filtered)?;
        Ok(())
    }

    fn transform_response<Counter>(
        &self,
        response: TransformResponse<'_, '_, '_>,
        logging: Logging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter,
    {
        *self.pre_call.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(logging.state.model_call_details.clone());
        transform_response(
            response.envelope,
            response.raw,
            response.request,
            logging,
            counter,
        )
    }
}

pub(crate) struct LoggerCall {
    pub(crate) input: Json,
    pub(crate) complete_input_dict: String,
}

#[derive(Default)]
pub(crate) struct Logger {
    pub(crate) calls: Mutex<Vec<LoggerCall>>,
}

impl StreamLogger for Logger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggerCall {
                input: call.input.clone(),
                complete_input_dict: call.complete_input_dict.to_owned(),
            });
    }
}

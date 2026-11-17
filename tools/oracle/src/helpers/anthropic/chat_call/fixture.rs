use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::anthropic_http_stream::error::{PostFailure, StatusFailure};
use betterlite_anthropic::anthropic_http_stream::transport::{
    LineChunks, StreamPost, StreamReply, StreamTransport,
};
use betterlite_anthropic::chat_call::config::{ChatConfig, TransformRequest};
use betterlite_anthropic::chat_call::error::ConfigError;
use betterlite_anthropic::chat_call::transport::{BufferedReply, ChatTransport};
use betterlite_anthropic::response::error::ResponseError;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{Logging, RawResponse, ResponseRequest, transform_response};
use betterlite_anthropic::stream_decode::TransportChunk;
use betterlite_core::api::anthropic_beta_config::matrix::{BetaMatrix, BetaNode};
use betterlite_core::api::anthropic_beta_filter::matrix_filter::filter_request_betas;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Json as PromptJson;
use futures_util::StreamExt as _;
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
    posts: Mutex<Vec<StreamPost>>,
}

impl FixtureTransport {
    pub(crate) const fn new(reply: FixtureReply, trip: Trip) -> Self {
        Self {
            reply,
            trip,
            posts: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn take_posts(&self) -> Vec<StreamPost> {
        std::mem::take(&mut *self.posts.lock().unwrap_or_else(PoisonError::into_inner))
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

    async fn post(
        &self,
        post: StreamPost,
    ) -> Result<StreamReply<FixtureFailure>, PostFailure<FixtureFailure>> {
        self.record(post);
        if let Some(failure) = self.failure() {
            return Err(failure);
        }
        let lines: Vec<String> = self.reply.lines.clone();
        let decoded: LineChunks<FixtureFailure> = futures_util::stream::iter(
            lines.into_iter().map(|line| Ok(TransportChunk::Text(line))),
        )
        .boxed();
        Ok(StreamReply {
            status_code: self.reply.status_code,
            headers: self.reply.headers.clone(),
            lines: decoded,
        })
    }
}

impl ChatTransport for FixtureTransport {
    async fn post_buffered(
        &self,
        post: StreamPost,
    ) -> Result<BufferedReply, PostFailure<FixtureFailure>> {
        self.record(post);
        if let Some(failure) = self.failure() {
            return Err(failure);
        }
        Ok(BufferedReply {
            status_code: self.reply.status_code,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        })
    }
}

pub(crate) struct Recorded {
    pub(crate) model: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) headers: IndexMap<String, String>,
}

pub(crate) struct FixtureConfig {
    body: IndexMap<String, Json>,
    failure: Option<String>,
    matrix: BetaMatrix,
    transforms: Mutex<Vec<Recorded>>,
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
            transforms: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn take_transforms(&self) -> Vec<Recorded> {
        std::mem::take(
            &mut *self
                .transforms
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    fn record(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        self.transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                model: request.model.to_owned(),
                messages: request.messages.to_vec(),
                optional_params: request.optional_params.clone(),
                litellm_params: request.litellm_params.clone(),
                headers: request.headers.values.clone(),
            });
        match &self.failure {
            Some(message) => Err(Box::new(FixtureFailure {
                message: message.clone(),
            })),
            None => Ok(self.body.clone()),
        }
    }
}

impl ChatConfig for FixtureConfig {
    fn transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        self.record(request)
    }

    fn async_transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send {
        ready(self.record(request))
    }

    // the beta dependency sits below the provider leaf, so this adapter owns the two-way JSON conversion
    fn filter_request_betas(
        &self,
        provider: &str,
        headers: &mut IndexMap<String, String>,
        body: &mut IndexMap<String, Json>,
    ) -> Result<(), ConfigError> {
        let mut prompt: IndexMap<String, PromptJson> =
            serde_json::from_value(serde_json::to_value(&*body)?)?;
        filter_request_betas(headers, &mut prompt, &self.matrix, provider)?;
        *body = serde_json::from_value(serde_json::to_value(&prompt)?)?;
        Ok(())
    }

    fn transform_response<Counter>(
        &self,
        envelope: &mut ResponseEnvelope,
        raw: &RawResponse<'_>,
        request: &ResponseRequest<'_>,
        logging: Logging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter,
    {
        transform_response(envelope, raw, request, logging, counter)
    }
}

pub(crate) struct LoggerCall {
    pub(crate) input: Json,
    pub(crate) original_response: String,
    pub(crate) complete_input_dict: String,
}

#[derive(Default)]
pub(crate) struct Logger {
    calls: Mutex<Vec<LoggerCall>>,
}

impl Logger {
    pub(crate) fn take(&self) -> Vec<LoggerCall> {
        std::mem::take(&mut *self.calls.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl StreamLogger for Logger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggerCall {
                input: call.input.clone(),
                original_response: call.original_response.to_owned(),
                complete_input_dict: call.complete_input_dict.to_owned(),
            });
    }
}

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::{Extra, Json};
use betterlite_tokenizer::engine::error::EngineError;
use futures_util::stream;
use futures_util::{Stream, StreamExt as _};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Number;

use crate::legacy_prediction::client::{
    EndpointPredictRequest, PredictionClient, PredictionStream, PublisherPredictRequest,
};
use crate::legacy_prediction::error::PredictionError;
use crate::legacy_prediction::execution::LegacyContext;
use crate::legacy_prediction::invocation::LegacyInvocation;
use crate::legacy_prediction::projection::{LocalEncoding, ProjectionServices};
use crate::legacy_prediction::records::{
    LegacyChunk, Prediction, PredictionsReply, ReplyMeta, TextReply, UsageMetadata,
};
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

pub(super) struct FakeEncoding {
    pub(super) counts: IndexMap<String, usize>,
}

impl LocalEncoding for FakeEncoding {
    fn encode(&self, text: &str) -> Result<usize, EngineError> {
        Ok(*self.counts.get(text).expect("a counted text"))
    }
}

pub(super) fn counts(pairs: &[(&str, usize)]) -> IndexMap<String, usize> {
    pairs
        .iter()
        .map(|(name, count)| ((*name).to_owned(), *count))
        .collect()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON")
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn float_number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite float"))
}

pub(super) fn options(pairs: &[(&str, Json)]) -> Extra {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn projection_services(encoder: &FakeEncoding) -> ProjectionServices<'_> {
    let encoding: &dyn LocalEncoding = encoder;
    ProjectionServices {
        now: 1_700_000_000,
        encoding,
        finish_reason: map_finish_reason,
    }
}

pub(super) fn legacy_context<'a, 'b>(
    services: &'a ProjectionServices<'b>,
) -> LegacyContext<'a, 'b> {
    LegacyContext {
        project: Some("proj"),
        location: Some("us-central1"),
        model_is_language: false,
        services,
    }
}

pub(super) fn usage_metadata() -> UsageMetadata {
    UsageMetadata {
        prompt_token_count: Some(1),
        candidates_token_count: Some(2),
        total_token_count: Some(3),
    }
}

pub(super) fn text_reply(text: &str) -> TextReply {
    TextReply {
        text: text.to_owned(),
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: Some(usage_metadata()),
        },
    }
}

pub(super) fn predictions_reply(values: &[&str]) -> PredictionsReply {
    PredictionsReply {
        predictions: values
            .iter()
            .map(|value| Prediction::Value(Json::Text((*value).to_owned())))
            .collect(),
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: Some(usage_metadata()),
        },
    }
}

pub(super) fn access() -> AccessToken {
    AccessToken {
        token: SecretString::from("resolved-token"),
        project: "resolved-project".to_owned(),
    }
}

pub(super) async fn access_lookup() -> Result<AccessToken, CredentialCacheError> {
    Ok(access())
}

pub(super) fn chat_invocation(options: Extra) -> LegacyInvocation {
    LegacyInvocation::Chat {
        model: "chat-bison".to_owned(),
        prompt: "hello".to_owned(),
        options,
    }
}

pub(super) fn text_invocation(options: Extra) -> LegacyInvocation {
    LegacyInvocation::Text {
        model: "text-bison".to_owned(),
        prompt: "hello".to_owned(),
        options,
    }
}

pub(super) fn custom_invocation(instances: Vec<Json>) -> LegacyInvocation {
    LegacyInvocation::CustomEndpoint {
        model: "model-garden".to_owned(),
        prompt: "hello".to_owned(),
        instances,
        options: Extra::new(),
    }
}

pub(super) fn private_invocation(instances: Vec<Json>) -> LegacyInvocation {
    LegacyInvocation::PrivateEndpoint {
        model: "1234".to_owned(),
        prompt: "hello".to_owned(),
        endpoint: "projects/proj/locations/us-central1/endpoints/1234".to_owned(),
        instances,
        options: Extra::new(),
    }
}

pub(super) fn vision_invocation() -> LegacyInvocation {
    LegacyInvocation::Vision {
        model: "gemini-2.5-flash".to_owned(),
        prompt: "hello".to_owned(),
    }
}

pub(super) fn empty_stream() -> PredictionStream {
    stream::empty().boxed()
}

#[derive(Clone, Debug)]
pub(super) enum Call {
    Message(PublisherPredictRequest),
    Text(PublisherPredictRequest),
    Endpoint(EndpointPredictRequest),
    MessageStream(PublisherPredictRequest),
    TextStream(PublisherPredictRequest),
}

#[derive(Default)]
pub(super) struct FixtureClient {
    calls: Mutex<Vec<Call>>,
    text: Mutex<Option<Result<TextReply, PredictionError>>>,
    predictions: Mutex<Option<Result<PredictionsReply, PredictionError>>>,
    stream: Mutex<Option<Result<PredictionStream, PredictionError>>>,
}

impl FixtureClient {
    pub(super) fn record(&self, call: Call) {
        self.calls.lock().expect("calls").push(call);
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls").clone()
    }

    pub(super) fn with_text(reply: TextReply) -> Self {
        let client = Self::default();
        *client.text.lock().expect("text") = Some(Ok(reply));
        client
    }

    pub(super) fn with_text_failure(error: PredictionError) -> Self {
        let client = Self::default();
        *client.text.lock().expect("text") = Some(Err(error));
        client
    }

    pub(super) fn with_endpoint(reply: PredictionsReply) -> Self {
        let client = Self::default();
        *client.predictions.lock().expect("predictions") = Some(Ok(reply));
        client
    }

    pub(super) fn with_endpoint_failure(error: PredictionError) -> Self {
        let client = Self::default();
        *client.predictions.lock().expect("predictions") = Some(Err(error));
        client
    }

    pub(super) fn with_stream(stream: PredictionStream) -> Self {
        let client = Self::default();
        *client.stream.lock().expect("stream") = Some(Ok(stream));
        client
    }

    pub(super) fn with_stream_failure(error: PredictionError) -> Self {
        let client = Self::default();
        *client.stream.lock().expect("stream") = Some(Err(error));
        client
    }
}

impl PredictionClient for FixtureClient {
    fn send_message_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<TextReply, PredictionError>> + Send {
        self.record(Call::Message(request.clone()));
        let reply = self
            .text
            .lock()
            .expect("text")
            .take()
            .expect("a fixture text reply");
        async move { reply }
    }

    fn predict_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<TextReply, PredictionError>> + Send {
        self.record(Call::Text(request.clone()));
        let reply = self
            .text
            .lock()
            .expect("text")
            .take()
            .expect("a fixture text reply");
        async move { reply }
    }

    fn predict_endpoint(
        &self,
        request: &EndpointPredictRequest,
    ) -> impl Future<Output = Result<PredictionsReply, PredictionError>> + Send {
        self.record(Call::Endpoint(request.clone()));
        let reply = self
            .predictions
            .lock()
            .expect("predictions")
            .take()
            .expect("a fixture prediction reply");
        async move { reply }
    }

    fn send_message_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<PredictionStream, PredictionError>> + Send {
        self.record(Call::MessageStream(request.clone()));
        let stream = self
            .stream
            .lock()
            .expect("stream")
            .take()
            .expect("a fixture stream");
        async move { stream }
    }

    fn predict_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<PredictionStream, PredictionError>> + Send {
        self.record(Call::TextStream(request.clone()));
        let stream = self
            .stream
            .lock()
            .expect("stream")
            .take()
            .expect("a fixture stream");
        async move { stream }
    }
}

pub(super) struct CancellingStream {
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) chunks: VecDeque<Result<LegacyChunk, PredictionError>>,
}

impl Stream for CancellingStream {
    type Item = Result<LegacyChunk, PredictionError>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.chunks.pop_front())
    }
}

impl Drop for CancellingStream {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

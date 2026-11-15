use std::future::{Future, Ready, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cache::evicted_client_closer::{CloseMethod, EvictableClient};
use betterlite_cache::llm_client_cache::LlmClientCacheError;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};
use betterlite_openai::chat::ChatMessage;
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::engine::error::EngineError;
use futures_util::StreamExt as _;
use futures_util::stream;
use secrecy::SecretString;
use uuid::Uuid;

use crate::gemini_options::defaults::GenerationDefaults;
use crate::legacy_prediction::client::{
    EndpointPredictRequest, PredictionClient, PredictionStream, PublisherPredictRequest,
};
use crate::legacy_prediction::error::PredictionError;
use crate::legacy_prediction::projection::{LegacyMessage, LocalEncoding, ProjectionServices};
use crate::legacy_prediction::records::{
    LegacyChunk, Prediction, PredictionsReply, ReplyMeta, TextReply, UsageMetadata,
};
use crate::legacy_vertex::LegacyCompletion;
use crate::legacy_vertex::LegacyDeps;
use crate::legacy_vertex::LegacyRequest;
use crate::legacy_vertex::cache::ClientCache;
use crate::legacy_vertex::complete;
use crate::legacy_vertex::plan::LegacyRoute;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;

pub(super) struct CountingEncoding;

impl LocalEncoding for CountingEncoding {
    fn encode(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
    }
}

// A pinned UUID4 spelling so a fake stream's response id is compared whole
pub(super) fn fixed_identifier() -> Uuid {
    Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("a pinned UUID")
}

pub(super) fn services(encoder: &CountingEncoding) -> ProjectionServices<'_> {
    let encoding: &dyn LocalEncoding = encoder;
    ProjectionServices {
        now: 1_700_000_000,
        encoding,
        finish_reason: map_finish_reason,
    }
}

pub(super) struct PlanFixture {
    pub(super) sets: ProviderModelSets,
    pub(super) defaults: GenerationDefaults,
    pub(super) project: Option<String>,
    pub(super) location: Option<String>,
}

impl PlanFixture {
    pub(super) fn new(groups: &[(ProviderModelSet, &str)]) -> Self {
        let sets = groups
            .iter()
            .fold(ProviderModelSets::default(), |mut sets, (set, model)| {
                sets.sets
                    .entry(*set)
                    .or_default()
                    .insert((*model).to_owned());
                sets
            });
        Self {
            sets,
            defaults: GenerationDefaults::default(),
            project: Some(String::from("proj")),
            location: Some(String::from("us-central1")),
        }
    }

    pub(super) fn deps<'fixture, 'encoding>(
        &'fixture self,
        services: &'fixture ProjectionServices<'encoding>,
    ) -> LegacyDeps<'fixture, 'encoding> {
        LegacyDeps {
            sets: &self.sets,
            defaults: &self.defaults,
            services,
            identifier: fixed_identifier,
            credentials: None,
            project: self.project.as_deref(),
            location: self.location.as_deref(),
        }
    }
}

pub(super) fn request<'request>(
    model: &'request str,
    messages: &'request [ChatMessage],
    options: Object,
    acompletion: bool,
) -> LegacyRequest<'request> {
    LegacyRequest {
        model,
        messages,
        options,
        acompletion,
    }
}

pub(super) fn options(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON option object")
}

pub(super) fn messages(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).expect("a JSON message list")
}

pub(super) fn route_options(route: &LegacyRoute) -> Option<&Extra> {
    match route {
        LegacyRoute::Chat { options }
        | LegacyRoute::Text { options, .. }
        | LegacyRoute::Custom { options, .. }
        | LegacyRoute::Private { options, .. } => Some(options),
        LegacyRoute::Vision => None,
    }
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

    pub(super) fn with_stream(chunks: Vec<LegacyChunk>) -> Self {
        let client = Self::default();
        let streamed = stream::iter(chunks.into_iter().map(Ok)).boxed();
        *client.stream.lock().expect("stream") = Some(Ok(streamed));
        client
    }

    pub(super) fn with_stream_failure(error: PredictionError) -> Self {
        let client = Self::default();
        *client.stream.lock().expect("stream") = Some(Err(error));
        client
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls").clone()
    }

    fn record(&self, call: Call) -> Option<Result<TextReply, PredictionError>> {
        self.calls.lock().expect("calls").push(call);
        self.text.lock().expect("text").take()
    }

    fn record_predictions(&self, call: Call) -> Option<Result<PredictionsReply, PredictionError>> {
        self.calls.lock().expect("calls").push(call);
        self.predictions.lock().expect("predictions").take()
    }

    fn record_stream(&self, call: Call) -> Option<Result<PredictionStream, PredictionError>> {
        self.calls.lock().expect("calls").push(call);
        self.stream.lock().expect("stream").take()
    }
}

fn replied<T>(
    reply: Option<Result<T, PredictionError>>,
) -> impl Future<Output = Result<T, PredictionError>> + Send
where
    T: Send,
{
    ready(reply.unwrap_or(Err(PredictionError::MissingPrediction)))
}

impl EvictableClient for FixtureClient {
    fn close_method(&self) -> Option<CloseMethod> {
        None
    }
}

impl PredictionClient for FixtureClient {
    fn send_message_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<TextReply, PredictionError>> + Send {
        replied(self.record(Call::Message(request.clone())))
    }

    fn predict_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<TextReply, PredictionError>> + Send {
        replied(self.record(Call::Text(request.clone())))
    }

    fn predict_endpoint(
        &self,
        request: &EndpointPredictRequest,
    ) -> impl Future<Output = Result<PredictionsReply, PredictionError>> + Send {
        replied(self.record_predictions(Call::Endpoint(request.clone())))
    }

    fn send_message_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<PredictionStream, PredictionError>> + Send {
        replied(self.record_stream(Call::MessageStream(request.clone())))
    }

    fn predict_streaming_async(
        &self,
        request: &PublisherPredictRequest,
    ) -> impl Future<Output = Result<PredictionStream, PredictionError>> + Send {
        replied(self.record_stream(Call::TextStream(request.clone())))
    }
}

pub(super) struct FixtureCache {
    keys: Mutex<Vec<String>>,
    client: Option<Arc<FixtureClient>>,
}

impl FixtureCache {
    pub(super) fn empty() -> Self {
        Self {
            keys: Mutex::new(Vec::new()),
            client: None,
        }
    }

    pub(super) fn with_client(client: Arc<FixtureClient>) -> Self {
        Self {
            keys: Mutex::new(Vec::new()),
            client: Some(client),
        }
    }

    pub(super) fn keys(&self) -> Vec<String> {
        self.keys.lock().expect("cache keys").clone()
    }
}

impl ClientCache for FixtureCache {
    type Client = FixtureClient;

    fn cached(&self, key: &str) -> Result<Option<Arc<FixtureClient>>, LlmClientCacheError> {
        self.keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        Ok(self.client.as_ref().map(Arc::clone))
    }
}

pub(super) fn access() -> AccessToken {
    AccessToken {
        token: SecretString::from("resolved-token"),
        project: String::from("resolved-project"),
    }
}

pub(super) fn token_lookup(
    _credentials: Option<&ConfiguredCredentials>,
    _project: Option<&str>,
) -> Ready<Result<AccessToken, CredentialCacheError>> {
    ready(Ok(access()))
}

pub(super) fn text_reply(text: &str) -> TextReply {
    TextReply {
        text: text.to_owned(),
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: Some(UsageMetadata {
                prompt_token_count: Some(1),
                candidates_token_count: Some(2),
                total_token_count: Some(3),
            }),
        },
    }
}

pub(super) fn predictions_reply(values: &[&str]) -> PredictionsReply {
    PredictionsReply {
        predictions: values
            .iter()
            .map(|value| Prediction::Value(Json::Text((*value).to_owned())))
            .collect(),
        meta: ReplyMeta::default(),
    }
}

pub(super) fn predictions_reply_with_usage(
    values: &[&str],
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
) -> PredictionsReply {
    let mut reply = predictions_reply(values);
    reply.meta.usage_metadata = Some(UsageMetadata {
        prompt_token_count: Some(prompt_tokens),
        candidates_token_count: Some(completion_tokens),
        total_token_count: Some(total_tokens),
    });
    reply
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON")
}

pub(super) fn content(message: &LegacyMessage) -> &str {
    let LegacyMessage::Content(text) = message else {
        panic!("a content message");
    };
    text
}

pub(super) fn chunk(text: &str) -> LegacyChunk {
    LegacyChunk {
        text: text.to_owned(),
    }
}

pub(super) async fn fake_stream_chunk(
    fixture: &PlanFixture,
    projection: &ProjectionServices<'_>,
    client: &FixtureClient,
) -> Json {
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request(
        "code-bison",
        &contents,
        options(r#"{"stream": true}"#),
        false,
    );
    let cache = FixtureCache::empty();
    let completion = complete(&cache, client, call, fixture.deps(projection), token_lookup)
        .await
        .expect("the fake stream");
    let LegacyCompletion::FakeStream(iterator) = completion else {
        panic!("the fake stream");
    };
    let chunks: Vec<Json> = Iterator::collect(iterator);
    assert_eq!(chunks.len(), 1);
    chunks.into_iter().next().expect("the one chunk")
}

#![cfg(test)]

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use betterlite_context::local_ttl_cache::CacheValue;

use crate::replay::{
    CacheHit, CacheHitMetadata, CachedDict, ReplayConversions, ReplayError, ReplayRequest,
    ReplayedBy, ResponseKind, TranscriptionHidden, replay_cached_result,
};
use crate::types::call_type::CachingSupportedCallType;

#[derive(Debug, Default)]
pub(crate) struct Model {
    pub(crate) cache_hit: bool,
}

impl CacheHitMetadata for Model {
    fn set_cache_hit(&mut self) {
        self.cache_hit = true;
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Recorded {
    pub(crate) model_kinds: Vec<ResponseKind>,
    pub(crate) model_dicts: Vec<CachedDict>,
    pub(crate) transcription_calls: u32,
    pub(crate) transcription_dict: Option<CachedDict>,
    pub(crate) transcription_hidden: Option<HiddenRecord>,
    pub(crate) responses_dict: Option<CachedDict>,
    pub(crate) responses_calls: u32,
    pub(crate) responses_stream_calls: u32,
    pub(crate) responses_stream_stamped: bool,
    pub(crate) responses_stream_call_type: Option<CachingSupportedCallType>,
    pub(crate) messages_probes: u32,
    pub(crate) messages_dict: Option<CachedDict>,
    pub(crate) messages_stream_calls: u32,
    pub(crate) wrapper_dict: Option<CachedDict>,
    pub(crate) wraps: Vec<WrapRecord>,
    pub(crate) metadata: Vec<MetadataRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HiddenRecord {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) cache_hit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WrapRecord {
    pub(crate) asynchronous: bool,
    pub(crate) model: String,
    pub(crate) custom_llm_provider: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetadataRecord {
    pub(crate) model: String,
    pub(crate) elapsed: Duration,
}

pub(crate) struct Fake {
    recorded: Arc<Mutex<Recorded>>,
    anthropic_events: bool,
}

impl Fake {
    pub(crate) fn new() -> Self {
        Self {
            recorded: Arc::new(Mutex::new(Recorded::default())),
            anthropic_events: false,
        }
    }

    pub(crate) fn with_anthropic_events() -> Self {
        Self {
            recorded: Arc::new(Mutex::new(Recorded::default())),
            anthropic_events: true,
        }
    }

    pub(crate) fn calls(&self) -> Recorded {
        lock(&self.recorded).clone()
    }
}

fn lock(recorded: &Mutex<Recorded>) -> MutexGuard<'_, Recorded> {
    match recorded.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Default for Fake {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplayConversions for Fake {
    type Response = Model;

    fn model_response(
        &self,
        cached: &CachedDict,
        kind: ResponseKind,
    ) -> Result<Self::Response, ReplayError> {
        let mut recorded = lock(&self.recorded);
        recorded.model_kinds.push(kind);
        recorded.model_dicts.push(cached.clone());
        drop(recorded);
        Ok(Model::default())
    }

    fn transcription_response(
        &self,
        cached: &CachedDict,
        hidden: &TranscriptionHidden<'_>,
    ) -> Result<Self::Response, ReplayError> {
        let mut recorded = lock(&self.recorded);
        recorded.transcription_calls = recorded.transcription_calls.saturating_add(1);
        recorded.transcription_dict = Some(cached.clone());
        recorded.transcription_hidden = Some(HiddenRecord {
            model: hidden.model.to_owned(),
            custom_llm_provider: hidden.custom_llm_provider.map(str::to_owned),
            cache_hit: hidden.cache_hit,
        });
        drop(recorded);
        Ok(Model::default())
    }

    fn responses_response(&self, cached: &CachedDict) -> Result<Self::Response, ReplayError> {
        let mut recorded = lock(&self.recorded);
        recorded.responses_calls = recorded.responses_calls.saturating_add(1);
        recorded.responses_dict = Some(cached.clone());
        drop(recorded);
        Ok(Model::default())
    }

    fn responses_stream(
        &self,
        response: Self::Response,
        call_type: CachingSupportedCallType,
    ) -> Result<Self::Response, ReplayError> {
        let mut recorded = lock(&self.recorded);
        recorded.responses_stream_calls = recorded.responses_stream_calls.saturating_add(1);
        recorded.responses_stream_stamped = response.cache_hit;
        recorded.responses_stream_call_type = Some(call_type);
        drop(recorded);
        Ok(Model::default())
    }

    fn anthropic_messages_stream(
        &self,
        cached: &CachedDict,
    ) -> Result<Option<Self::Response>, ReplayError> {
        let mut recorded = lock(&self.recorded);
        recorded.messages_probes = recorded.messages_probes.saturating_add(1);
        recorded.messages_dict = Some(cached.clone());
        let stream = self.anthropic_events.then(Model::default);
        if stream.is_some() {
            recorded.messages_stream_calls = recorded.messages_stream_calls.saturating_add(1);
        }
        drop(recorded);
        Ok(stream)
    }

    fn stream_wrapper(
        &self,
        cached: &CachedDict,
        asynchronous: bool,
        model: &str,
        custom_llm_provider: &str,
    ) -> Result<Self::Response, ReplayError> {
        let mut recorded = lock(&self.recorded);
        recorded.wrapper_dict = Some(cached.clone());
        recorded.wraps.push(WrapRecord {
            asynchronous,
            model: model.to_owned(),
            custom_llm_provider: custom_llm_provider.to_owned(),
        });
        drop(recorded);
        Ok(Model::default())
    }

    fn update_response_metadata(
        &self,
        _result: &mut ReplayedBy<Self>,
        model: &str,
        elapsed: Duration,
    ) {
        let mut recorded = lock(&self.recorded);
        recorded.metadata.push(MetadataRecord {
            model: model.to_owned(),
            elapsed,
        });
    }
}

pub(crate) fn text(value: &str) -> CacheValue {
    CacheValue::Text(value.to_owned())
}

pub(crate) fn dict(entries: Vec<(&str, CacheValue)>) -> CacheValue {
    CacheValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(crate) fn dict_of(value: CacheValue) -> CachedDict {
    let CacheValue::Object(dict) = value else {
        panic!("the fixture is not an object");
    };
    dict
}

pub(crate) fn chat_completion_dict() -> CacheValue {
    dict(vec![
        ("id", text("chatcmpl-abc")),
        ("object", text("chat.completion")),
        ("model", text("gpt-4o")),
        ("choices", CacheValue::Array(Vec::new())),
    ])
}

pub(crate) fn text_completion_dict() -> CacheValue {
    dict(vec![
        ("id", text("cmpl-1")),
        ("object", text("text_completion")),
        ("choices", CacheValue::Array(Vec::new())),
    ])
}

pub(crate) fn embedding_dict() -> CacheValue {
    dict(vec![
        ("object", text("list")),
        ("model", text("text-embedding-3-small")),
        ("data", CacheValue::Array(Vec::new())),
    ])
}

pub(crate) fn rerank_dict() -> CacheValue {
    dict(vec![
        ("id", text("rerank-1")),
        ("results", CacheValue::Array(Vec::new())),
    ])
}

pub(crate) fn transcription_dict() -> CacheValue {
    dict(vec![
        ("text", text("hello there")),
        ("model", text("gpt-4o-transcribe")),
    ])
}

pub(crate) fn responses_dict() -> CacheValue {
    dict(vec![
        ("id", text("resp_1")),
        ("object", text("response")),
        ("output", CacheValue::Array(Vec::new())),
    ])
}

pub(crate) fn messages_dict() -> CacheValue {
    dict(vec![
        ("id", text("msg_1")),
        ("type", text("message")),
        ("content", CacheValue::Array(Vec::new())),
    ])
}

pub(crate) fn fixture(call_type: CachingSupportedCallType) -> CacheValue {
    match call_type {
        CachingSupportedCallType::Completion | CachingSupportedCallType::Acompletion => {
            chat_completion_dict()
        }
        CachingSupportedCallType::TextCompletion | CachingSupportedCallType::AtextCompletion => {
            text_completion_dict()
        }
        CachingSupportedCallType::Embedding | CachingSupportedCallType::Aembedding => {
            embedding_dict()
        }
        CachingSupportedCallType::Rerank | CachingSupportedCallType::Arerank => rerank_dict(),
        CachingSupportedCallType::Transcription | CachingSupportedCallType::Atranscription => {
            transcription_dict()
        }
        CachingSupportedCallType::Responses | CachingSupportedCallType::Aresponses => {
            responses_dict()
        }
        CachingSupportedCallType::AnthropicMessages
        | CachingSupportedCallType::AanthropicMessages => messages_dict(),
    }
}

pub(crate) fn non_stream_request() -> ReplayRequest<'static> {
    ReplayRequest {
        stream: false,
        converted_stream_requested: false,
        agentic_loop_depth: 0,
        model: "gpt-4o",
        custom_llm_provider: None,
        elapsed: Duration::from_millis(7),
    }
}

pub(crate) fn stream_request() -> ReplayRequest<'static> {
    ReplayRequest {
        stream: true,
        ..non_stream_request()
    }
}

pub(crate) fn converted_stream_request(depth: u32) -> ReplayRequest<'static> {
    ReplayRequest {
        converted_stream_requested: true,
        agentic_loop_depth: depth,
        ..non_stream_request()
    }
}

pub(crate) fn convert(
    cached: CacheValue,
    call_type: CachingSupportedCallType,
    request: &ReplayRequest<'_>,
    fake: &Fake,
) -> Result<ReplayedBy<Fake>, ReplayError> {
    replay_cached_result(CacheHit::Stored(cached), call_type, request, fake)
}

pub(crate) fn replay(
    call_type: CachingSupportedCallType,
    request: &ReplayRequest<'_>,
    fake: &Fake,
) -> ReplayedBy<Fake> {
    convert(fixture(call_type), call_type, request, fake).unwrap()
}

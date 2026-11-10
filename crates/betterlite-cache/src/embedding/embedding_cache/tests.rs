#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::{Extra, Json};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;
use rstest::rstest;
use serde_json::Number;

use crate::backend::{BackendError, CacheBackend, CacheResult, Ttl};
use crate::cache_key::derive::get_cache_key;
use crate::cache_key::request::{CacheKeyRequest, KeyParam, RequestCacheControl};
use crate::cache_key::settings::CacheKeySettings;
use crate::cache_key::value::KeyValue;
use crate::codec::CacheCodec;
use crate::embedding::embedding_cache::pipeline::{
    EmbeddingCacheStore, NativeCacheRequest, NativeEmbeddingStore, async_add_cache_pipeline,
};
use crate::embedding::embedding_cache::{
    EmbeddingCacheEntry, EmbeddingCacheError, add_embedding_entry_to_cache,
    convert_to_cached_embedding, tokens,
};
use crate::embedding::response::{EmbeddingRecord, EmbeddingResponse, EmbeddingUsage};
use crate::envelope::backend::StoredBackend;
use crate::envelope::facade::{EnvelopeSettings, ResponseCache};
use crate::envelope::request::EnvelopeRequest;
use crate::envelope::{RESPONSE_FIELD, TIMESTAMP_FIELD};
use crate::types::cache_type::{CacheMode, LiteLlmCacheType};
use crate::types::control::DynamicCacheControl;
use crate::types::embedding::{CachedEmbedding, EMBEDDING_CACHE_FORMAT_VERSION, EmbeddingPayload};
use crate::types::semantic::SemanticCacheScope;

const NOW: f64 = 1_700_000_000.0;

type Stored = (String, CacheValue, Ttl);
type PipelineWrite = (Vec<(String, CacheValue)>, Ttl);

#[derive(Default)]
struct RecordingLogger {
    records: Mutex<Vec<(String, String)>>,
}

impl RecordingLogger {
    fn matching(&self, level: &str, needle: &str) -> usize {
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .filter(|(recorded, text)| recorded == level && text.contains(needle))
            .count()
    }
}

impl LoggerSink for RecordingLogger {
    fn debug(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(("DEBUG".to_owned(), payload_text(payload)));
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, payload: &Payload) -> bool {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(("ERROR".to_owned(), payload_text(payload)));
        true
    }

    fn debug_enabled(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct RecordingConsole;

impl ConsoleSink for RecordingConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

fn payload_text(payload: &Payload) -> String {
    match payload {
        Payload::String(text) => text.clone(),
        Payload::Null
        | Payload::Bool(_)
        | Payload::Number(_)
        | Payload::Array(_)
        | Payload::Object(_) => String::new(),
    }
}

type Hooks = VerboseHooks<RecordingLogger, RecordingConsole>;

fn hooks() -> Hooks {
    VerboseHooks {
        logger: RecordingLogger::default(),
        console: RecordingConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("fixture redactor")),
    }
}

#[derive(Default)]
struct RecordingCache {
    failure: bool,
    sets: Mutex<Vec<Stored>>,
}

impl RecordingCache {
    fn sets(&self) -> Vec<Stored> {
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl CacheBackend for RecordingCache {
    fn default_ttl(&self) -> i64 {
        60
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        if self.failure {
            return Err(BackendError::Unsupported("async_set_cache"));
        }
        self.sets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl));
        Ok(())
    }
}

#[derive(Default)]
struct RecordingStore {
    failure: bool,
    writes: Mutex<Vec<PipelineWrite>>,
}

impl RecordingStore {
    fn writes(&self) -> Vec<PipelineWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl EmbeddingCacheStore for RecordingStore {
    async fn async_set_cache_pipeline(
        &self,
        cache_list: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> CacheResult<()> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((cache_list, ttl));
        if self.failure {
            return Err(BackendError::Unsupported("async_set_cache_pipeline"));
        }
        Ok(())
    }
}

#[derive(Default)]
struct RecordingNative {
    failure: bool,
    skip_last: bool,
    calls: Mutex<usize>,
    requests: Mutex<Vec<NativeCacheRequest>>,
    responses: Mutex<Vec<CacheValue>>,
}

impl RecordingNative {
    fn requests(&self) -> Vec<NativeCacheRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn responses(&self) -> Vec<CacheValue> {
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl NativeEmbeddingStore for RecordingNative {
    fn request(&self, cache_key: &str, _request: &EnvelopeRequest) -> Option<NativeCacheRequest> {
        let skip = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            *calls = calls.saturating_add(1);
            self.skip_last && *calls > 1
        };
        if skip {
            return None;
        }
        Some(NativeCacheRequest {
            cache_key: cache_key.to_owned(),
        })
    }

    async fn async_store_batch(
        &self,
        requests: Vec<NativeCacheRequest>,
        responses: Vec<CacheValue>,
    ) -> CacheResult<()> {
        if self.failure {
            return Err(BackendError::Unsupported("async_store_batch"));
        }
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(requests);
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(responses);
        Ok(())
    }
}

fn key_settings(mode: CacheMode) -> CacheKeySettings {
    CacheKeySettings {
        cache_type: LiteLlmCacheType::Local,
        mode,
        namespace: None,
        semantic_cache_scope: SemanticCacheScope::Key,
        include_provider_params: false,
    }
}

fn settings(ttl: Option<f64>) -> EnvelopeSettings {
    EnvelopeSettings {
        ttl,
        redis_flush_size: None,
    }
}

fn cache(
    backend: Arc<RecordingCache>,
    mode: CacheMode,
    settings: EnvelopeSettings,
) -> ResponseCache<RecordingLogger, RecordingConsole> {
    ResponseCache {
        backend: Arc::new(StoredBackend { backend }),
        key_settings: key_settings(mode),
        settings,
        hooks: hooks(),
        clock: Arc::new(|| NOW),
    }
}

fn request(params: Vec<KeyParam>) -> EnvelopeRequest {
    EnvelopeRequest {
        cache_key: None,
        key: CacheKeyRequest {
            params,
            metadata: None,
            litellm_metadata: None,
            litellm_params: None,
            cache_control: None,
        },
        control: DynamicCacheControl::default(),
    }
}

fn param(name: &str, value: KeyValue) -> KeyParam {
    KeyParam {
        name: name.to_owned(),
        value,
    }
}

fn text(value: &str) -> KeyValue {
    KeyValue::String(value.to_owned())
}

fn inputs(values: &[&str]) -> KeyValue {
    KeyValue::Array(values.iter().map(|value| text(value)).collect())
}

fn response(count: usize, prompt_tokens: Option<i64>, details: Option<Extra>) -> EmbeddingResponse {
    EmbeddingResponse {
        model: Some("text-embedding-3-small".to_owned()),
        data: (0..count)
            .map(|index| EmbeddingRecord {
                embedding: Some(EmbeddingPayload::Vector(vec![Number::from(
                    u64::try_from(index).expect("a small index"),
                )])),
                index: Some(i64::try_from(index).expect("a small index")),
                object: Some("embedding".to_owned()),
            })
            .collect(),
        usage: Some(EmbeddingUsage {
            prompt_tokens,
            prompt_tokens_details: details,
        }),
    }
}

fn integer(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn decimal(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("a finite fixture"))
}

fn details(pairs: &[(&str, Json)]) -> Extra {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn cached(entry: &EmbeddingCacheEntry) -> CachedEmbedding {
    CacheCodec::decode(Some(&entry.cached_embedding)).expect("the entry encodes a cached embedding")
}

fn assert_envelope(entry: &EmbeddingCacheEntry) {
    match &entry.cached_data {
        CacheValue::Object(fields) => {
            assert_eq!(fields.get(TIMESTAMP_FIELD), Some(&CacheValue::Float(NOW)));
            assert_eq!(fields.get(RESPONSE_FIELD), Some(&entry.cached_embedding));
        }
        CacheValue::Credential(_)
        | CacheValue::Bytes(_)
        | CacheValue::Null
        | CacheValue::Boolean(_)
        | CacheValue::Integer(_)
        | CacheValue::Float(_)
        | CacheValue::Text(_)
        | CacheValue::Array(_) => panic!("the entry is not an envelope"),
    }
}

#[test]
fn convert_to_cached_embedding_builds_version_two_record() {
    let record = EmbeddingRecord {
        embedding: Some(EmbeddingPayload::Text("base64".to_owned())),
        index: Some(3),
        object: Some("embedding".to_owned()),
    };
    let prompt_details = details(&[("image_tokens", integer(2))]);
    let converted = convert_to_cached_embedding(
        &record,
        Some("text-embedding-3-small"),
        Some(4),
        Some(prompt_details.clone()),
    );
    assert_eq!(converted.embedding, record.embedding);
    assert_eq!(converted.index, Some(3));
    assert_eq!(converted.object.as_deref(), Some("embedding"));
    assert_eq!(converted.model.as_deref(), Some("text-embedding-3-small"));
    assert_eq!(converted.prompt_tokens, Some(4));
    assert_eq!(converted.prompt_tokens_details, Some(prompt_details));
    assert_eq!(converted.format_version, 2);
    assert_eq!(converted.format_version, EMBEDDING_CACHE_FORMAT_VERSION);
}

#[test]
fn convert_to_cached_embedding_keeps_absent_fields_none() {
    let record = EmbeddingRecord {
        embedding: None,
        index: None,
        object: None,
    };
    let converted = convert_to_cached_embedding(&record, None, None, None);
    assert_eq!(
        converted,
        CachedEmbedding {
            embedding: None,
            index: None,
            object: None,
            model: None,
            prompt_tokens: None,
            prompt_tokens_details: None,
            format_version: EMBEDDING_CACHE_FORMAT_VERSION,
        }
    );
}

#[rstest]
#[case::single_item_keeps_total(1, 0, 10, 10)]
#[case::even_division(2, 0, 10, 5)]
#[case::even_division_late_index(2, 1, 10, 5)]
#[case::remainder_goes_to_earliest(3, 0, 10, 4)]
#[case::remainder_skips_late_indices(3, 1, 10, 3)]
#[case::remainder_last_index(3, 2, 10, 3)]
#[case::empty_data_keeps_total(0, 0, 10, 10)]
fn per_item_prompt_tokens_distributes(
    #[case] count: usize,
    #[case] idx: usize,
    #[case] total: i64,
    #[case] expected: i64,
) {
    let result = response(count, Some(total), None);
    assert_eq!(tokens::per_item_prompt_tokens(&result, idx), Some(expected));
}

#[test]
fn per_item_prompt_tokens_without_usage_is_none() {
    let mut result = response(2, Some(10), None);
    result.usage = None;
    assert_eq!(tokens::per_item_prompt_tokens(&result, 0), None);
}

#[test]
fn per_item_prompt_tokens_without_total_is_none() {
    let result = response(2, None, None);
    assert_eq!(tokens::per_item_prompt_tokens(&result, 0), None);
}

#[rstest]
#[case::first_index(0, -2)]
#[case::second_index(1, -3)]
fn per_item_prompt_tokens_floors_like_python(#[case] idx: usize, #[case] expected: i64) {
    let result = response(2, Some(-5), None);
    assert_eq!(tokens::per_item_prompt_tokens(&result, idx), Some(expected));
}

#[test]
fn per_item_details_without_usage_is_none() {
    let mut result = response(2, Some(10), Some(details(&[("cached_tokens", integer(4))])));
    result.usage = None;
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_without_details_is_none() {
    let result = response(2, Some(10), None);
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_empty_map_is_none() {
    let result = response(2, Some(10), Some(Extra::new()));
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_all_null_is_none() {
    let result = response(1, Some(10), Some(details(&[("cached_tokens", Json::Null)])));
    assert_eq!(tokens::per_item_prompt_tokens_details(&result, 0), None);
}

#[test]
fn per_item_details_null_fields_are_dropped() {
    let result = response(
        1,
        Some(10),
        Some(details(&[
            ("cached_tokens", integer(4)),
            ("image_tokens", Json::Null),
        ])),
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(details(&[("cached_tokens", integer(4))]))
    );
}

#[test]
fn per_item_details_single_item_keeps_every_value() {
    let prompt_details = details(&[
        ("cached_tokens", integer(4)),
        ("ratio", decimal(1.5)),
        ("note", Json::Text("x".to_owned())),
    ]);
    let result = response(1, Some(10), Some(prompt_details.clone()));
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(prompt_details)
    );
}

#[rstest]
#[case::first_index(0, 3, 1)]
#[case::second_index(1, 2, 1)]
#[case::last_index(2, 2, 0)]
fn per_item_details_distributes_integers(
    #[case] idx: usize,
    #[case] expected_cached: i64,
    #[case] expected_image: i64,
) {
    let result = response(
        3,
        Some(10),
        Some(details(&[
            ("cached_tokens", integer(7)),
            ("image_tokens", integer(2)),
        ])),
    );
    let distributed =
        tokens::per_item_prompt_tokens_details(&result, idx).expect("the details distribute");
    assert_eq!(
        distributed.get("cached_tokens"),
        Some(&integer(expected_cached))
    );
    assert_eq!(
        distributed.get("image_tokens"),
        Some(&integer(expected_image))
    );
}

#[test]
fn per_item_details_divides_floats() {
    let result = response(2, Some(10), Some(details(&[("ratio", decimal(1.5))])));
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(details(&[("ratio", decimal(0.75))]))
    );
}

#[test]
fn per_item_details_passes_other_values_through() {
    let prompt_details = details(&[
        ("note", Json::Text("x".to_owned())),
        ("flag", Json::Bool(true)),
        ("nothing", Json::Null),
    ]);
    let result = response(2, Some(10), Some(prompt_details));
    let expected = details(&[
        ("note", Json::Text("x".to_owned())),
        ("flag", Json::Bool(true)),
    ]);
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(expected.clone())
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(expected)
    );
}

#[test]
fn per_item_details_negative_integer_floors() {
    let result = response(
        2,
        Some(10),
        Some(details(&[("cached_tokens", integer(-5))])),
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 0),
        Some(details(&[("cached_tokens", integer(-2))]))
    );
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 1),
        Some(details(&[("cached_tokens", integer(-3))]))
    );
}

#[test]
fn per_item_details_empty_data_keeps_every_value() {
    let prompt_details = details(&[("cached_tokens", integer(7))]);
    let result = response(0, Some(10), Some(prompt_details.clone()));
    assert_eq!(
        tokens::per_item_prompt_tokens_details(&result, 4),
        Some(prompt_details)
    );
}

#[test]
fn add_embedding_entry_derives_key_and_envelope() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("model", text("text-embedding-3-small"))]);
    let result = response(1, Some(10), None);
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    assert!(!entry.cache_key.is_empty());
    assert_eq!(
        cache_request.cache_key.as_deref(),
        Some(entry.cache_key.as_str())
    );
    assert_eq!(entry.ttl, Some(600.0_f64));
    assert_envelope(&entry);
    let stored = cached(&entry);
    assert_eq!(stored.model.as_deref(), Some("text-embedding-3-small"));
    assert_eq!(stored.prompt_tokens, Some(10));
    assert_eq!(stored.format_version, EMBEDDING_CACHE_FORMAT_VERSION);
}

#[test]
fn add_embedding_entry_derives_the_key_from_the_item_input() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![
        param("model", text("m")),
        param("input", inputs(&["hello", "world"])),
    ]);
    let result = response(2, Some(10), None);
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    let mut reference = request(vec![
        param("model", text("m")),
        param("input", text("hello")),
    ]);
    let expected = get_cache_key(&mut reference.key, &cache.key_settings, &cache.hooks)
        .expect("the reference key derives");
    assert_eq!(entry.cache_key, expected);
}

#[test]
fn add_embedding_entry_restores_the_request_input() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let original = inputs(&["hello", "world"]);
    let mut cache_request = request(vec![
        param("model", text("text-embedding-3-small")),
        param("input", original.clone()),
    ]);
    let result = response(1, Some(6), None);
    add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
        .expect("the entry is built");
    let stored_input = cache_request
        .key
        .params
        .iter()
        .find(|param| param.name == "input")
        .map(|param| param.value.clone());
    assert_eq!(stored_input, Some(original));
}

#[test]
fn add_embedding_entry_appends_the_input_for_the_key_only() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("model", text("m"))]);
    let result = response(1, Some(6), None);
    add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
        .expect("the entry is built");
    assert!(
        cache_request
            .key
            .params
            .iter()
            .all(|param| param.name != "input")
    );
}

#[test]
fn add_embedding_entry_uses_item_tokens_and_details() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(
        2,
        Some(10),
        Some(details(&[
            ("cached_tokens", integer(3)),
            ("image_tokens", integer(2)),
        ])),
    );
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    let stored = cached(&entry);
    assert_eq!(stored.prompt_tokens, Some(5));
    assert_eq!(
        stored.prompt_tokens_details,
        Some(details(&[
            ("cached_tokens", integer(2)),
            ("image_tokens", integer(1)),
        ]))
    );
}

#[test]
fn add_embedding_entry_missing_item_errors() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    let error =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 1)
            .expect_err("the response has no second item");
    assert!(matches!(
        error,
        EmbeddingCacheError::MissingResponseItem { index: 1 }
    ));
}

#[test]
fn add_embedding_entry_control_ttl_wins() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    cache_request.control.ttl = Some(30);
    let result = response(1, Some(10), None);
    let entry =
        add_embedding_entry_to_cache(&cache, &mut cache_request, &result, &text("hello"), 0)
            .expect("the entry is built");
    assert_eq!(entry.ttl, Some(30.0_f64));
}

#[tokio::test]
async fn pipeline_skips_when_cache_is_disabled() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOff,
        settings(None),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert!(cache_request.cache_key.is_none());
}

#[tokio::test]
async fn pipeline_writes_when_opted_in() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOff,
        settings(None),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    cache_request.key.cache_control = Some(RequestCacheControl {
        namespace: None,
        use_cache: true,
    });
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert_eq!(recording.sets().len(), 1);
}

#[tokio::test]
async fn pipeline_skips_more_inputs_than_embeddings() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("DEBUG", "skipping embedding cache write"),
        1
    );
}

#[tokio::test]
async fn pipeline_skips_more_embeddings_than_inputs() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("DEBUG", "skipping embedding cache write"),
        1
    );
}

#[tokio::test]
async fn pipeline_writes_every_entry_with_the_settings_ttl() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    let writes = recording.sets();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes.first().map(|write| write.2), Some(Some(600.0_f64)));
    assert_eq!(writes.get(1).map(|write| write.2), Some(Some(600.0_f64)));
    assert_ne!(
        writes.first().map(|write| write.0.clone()),
        writes.get(1).map(|write| write.0.clone())
    );
    assert_eq!(
        cache_request.cache_key.as_deref(),
        writes.last().map(|write| write.0.as_str())
    );
}

#[tokio::test]
async fn pipeline_control_ttl_overrides_settings_ttl() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", text("hello"))]);
    cache_request.control.ttl = Some(30);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert_eq!(
        recording.sets().first().map(|write| write.2),
        Some(Some(30.0_f64))
    );
}

#[tokio::test]
async fn pipeline_uses_the_dynamic_store_when_injected() {
    let recording = Arc::new(RecordingCache::default());
    let dynamic = RecordingStore::default();
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, Some(&dynamic), None, &mut cache_request, &result).await;
    let writes = dynamic.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes.first().map(|write| write.0.len()), Some(2));
    assert_eq!(writes.first().map(|write| write.1), Some(Some(600.0_f64)));
    assert!(recording.sets().is_empty());
}

#[tokio::test]
async fn pipeline_uses_native_batch_storage_when_configured() {
    let recording = Arc::new(RecordingCache::default());
    let dynamic = RecordingStore::default();
    let native = RecordingNative::default();
    let cache = cache(
        Arc::clone(&recording),
        CacheMode::DefaultOn,
        settings(Some(600.0_f64)),
    );
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(
        &cache,
        Some(&dynamic),
        Some(&native),
        &mut cache_request,
        &result,
    )
    .await;
    let requests = native.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(native.responses().len(), 2);
    assert!(dynamic.writes().is_empty());
    assert!(recording.sets().is_empty());
}

#[tokio::test]
async fn pipeline_native_filters_missing_requests() {
    let recording = Arc::new(RecordingCache::default());
    let native = RecordingNative {
        failure: false,
        skip_last: true,
        calls: Mutex::new(0),
        requests: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    };
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", inputs(&["hello", "world"]))]);
    let result = response(2, Some(10), None);
    async_add_cache_pipeline(&cache, None, Some(&native), &mut cache_request, &result).await;
    assert_eq!(native.requests().len(), 1);
    assert_eq!(native.responses().len(), 1);
}

#[tokio::test]
async fn pipeline_native_failure_is_logged() {
    let recording = Arc::new(RecordingCache::default());
    let native = RecordingNative {
        failure: true,
        skip_last: false,
        calls: Mutex::new(0),
        requests: Mutex::new(Vec::new()),
        responses: Mutex::new(Vec::new()),
    };
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, Some(&native), &mut cache_request, &result).await;
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
}

#[tokio::test]
async fn pipeline_store_failure_is_logged() {
    let recording = Arc::new(RecordingCache {
        failure: true,
        sets: Mutex::new(Vec::new()),
    });
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
}

#[tokio::test]
async fn pipeline_dynamic_failure_is_logged() {
    let recording = Arc::new(RecordingCache::default());
    let dynamic = RecordingStore {
        failure: true,
        writes: Mutex::new(Vec::new()),
    };
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("input", text("hello"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, Some(&dynamic), None, &mut cache_request, &result).await;
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
    assert!(recording.sets().is_empty());
}

#[tokio::test]
async fn pipeline_missing_input_is_logged() {
    let recording = Arc::new(RecordingCache::default());
    let cache = cache(Arc::clone(&recording), CacheMode::DefaultOn, settings(None));
    let mut cache_request = request(vec![param("model", text("m"))]);
    let result = response(1, Some(10), None);
    async_add_cache_pipeline(&cache, None, None, &mut cache_request, &result).await;
    assert!(recording.sets().is_empty());
    assert_eq!(
        cache
            .hooks
            .logger
            .matching("ERROR", "exception in add_cache"),
        1
    );
}

#![cfg(test)]

use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::provider_metadata::{
    ChatConfigSelection, ProjectionError, ProviderMetadataContext, ProviderMetadataProjection,
    ProviderResolution, apply_provider_metadata,
};
use super::records::Chunk;
use super::response::{
    BaseChoice, BaseMessage, BaseResponse, ChoiceProviderFields, apply_hidden_params,
    build_base_response,
};
use super::{StreamBaseAssemblyError, StreamedChunks};
use crate::stream_chunk_shaping::records::{JsonObject, JsonValue, Usage};

#[rstest]
#[case::conditional_sort_original_first(conditional_sort_original_first)]
#[case::base_metadata(base_metadata)]
#[case::finish_and_hidden(finish_and_hidden)]
#[case::assembled_provider_metadata(assembled_provider_metadata)]
fn stream_base_assembly_cases(#[case] verify: fn()) {
    verify();
}

fn chunks(source: &str) -> Vec<Chunk> {
    serde_json::from_str(source).unwrap()
}

fn chunk(source: &str) -> Chunk {
    serde_json::from_str(source).unwrap()
}

fn object(source: &str) -> JsonObject {
    serde_json::from_str(source).unwrap()
}

fn ids(chunks: &[Chunk]) -> Vec<&str> {
    chunks
        .iter()
        .map(|chunk| chunk.id.as_deref().unwrap_or_default())
        .collect()
}

fn base_for(source: &str) -> BaseResponse {
    build_base_response(&StreamedChunks::new(chunks(source)).unwrap()).unwrap()
}

fn conditional_sort_original_first() {
    sorting_order();
    sort_needs_first_created_at();
}

fn sorting_order() {
    let sorted = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":{"created_at":3}},{"id":"b","_hidden_params":{"created_at":1}},
            {"id":"c"},{"id":"d","_hidden_params":"junk"}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&sorted.chunks), ["b", "a", "c", "d"]);
    assert_eq!(sorted.first_chunk.id.as_deref(), Some("a"));

    let stable = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":{"created_at":1}},{"id":"b","_hidden_params":{"created_at":1}},{"id":"c"}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&stable.chunks), ["a", "b", "c"]);
}

fn sort_needs_first_created_at() {
    // created_at 0 does not turn sorting on
    let unsorted = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":{"created_at":0}},{"id":"b","_hidden_params":{"created_at":-2}}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&unsorted.chunks), ["a", "b"]);

    let non_dict = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":"junk"},{"id":"b","_hidden_params":{"created_at":1}}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&non_dict.chunks), ["a", "b"]);

    let null_hidden = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":null},{"id":"b","_hidden_params":{"created_at":1}}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&null_hidden.chunks), ["a", "b"]);

    // a truthy non-number still turns sorting on, and its key sorts last
    let text_timestamp = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":{"created_at":"soon"}},{"id":"b","_hidden_params":{"created_at":1}}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&text_timestamp.chunks), ["b", "a"]);

    // an empty list or object is not a timestamp
    let empty_array = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":{"created_at":[]}},{"id":"b","_hidden_params":{"created_at":1}}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&empty_array.chunks), ["a", "b"]);

    let empty_object = StreamedChunks::new(chunks(
        r#"[{"id":"a","_hidden_params":{"created_at":{}}},{"id":"b","_hidden_params":{"created_at":1}}]"#,
    ))
    .unwrap();
    assert_eq!(ids(&empty_object.chunks), ["a", "b"]);

    assert_eq!(
        StreamedChunks::new(Vec::new()),
        Err(StreamBaseAssemblyError::NoChunks)
    );
}

fn base_metadata() {
    base_metadata_identity();
    base_metadata_sorted_first();
    base_metadata_defaults();
    base_needs_first_fields();
}

fn base_metadata_identity() {
    let response = base_for(
        r#"[{"object":"chat.completion.chunk","id":"","created":7,"model":"orig","system_fingerprint":"fp","choices":[]},
            {"object":"chat.completion.chunk","id":"id2","model":"real-1","choices":[{"delta":{"role":"user"},"finish_reason":null}]},
            {"object":"chat.completion.chunk","id":"id3","model":"real-2","service_tier":"t1","choices":[{"delta":{"role":"tool"}}]},
            {"object":"chat.completion.chunk","service_tier":"t2"},
            {"object":"chat.completion.chunk","service_tier":""}]"#,
    );
    assert_eq!(response.id, "id2");
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.created, 7);
    assert_eq!(response.model, "real-1");
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp"));
    assert_eq!(response.service_tier.as_deref(), Some("t2"));
    assert_eq!(
        response.choices,
        vec![BaseChoice {
            index: 0,
            message: BaseMessage {
                role: "user".to_owned(),
                content: Some(String::new()),
            },
            finish_reason: "stop".to_owned(),
            provider_specific_fields: None,
        }]
    );
    assert_eq!(response.usage, Usage::default());
}

fn base_metadata_sorted_first() {
    // the sort reorders the chunks, but object, created, model source and fingerprint come from the original first
    let response = base_for(
        r#"[{"object":"chat.completion.chunk","id":"a","created":7,"model":"orig","system_fingerprint":"fp","choices":[],"_hidden_params":{"created_at":2}},
            {"object":"chat.completion.chunk","id":"b","created":9,"model":"orig2","choices":[],"_hidden_params":{"created_at":1}}]"#,
    );
    assert_eq!(response.created, 7);
    assert_eq!(response.system_fingerprint.as_deref(), Some("fp"));
    assert_eq!(response.model, "orig2");
    assert_eq!(response.id, "b");
    assert_eq!(response.hidden_params, object(r#"{"created_at":2}"#));
}

fn base_metadata_defaults() {
    // the first chunk with choices wins and only its first choice is read; an empty role falls back
    let role = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"m","choices":[]},
            {"object":"chat.completion.chunk","created":1,"model":"m","choices":[{"delta":{"role":""}},{"delta":{"role":"tool"}}]},
            {"object":"chat.completion.chunk","created":1,"model":"m","choices":[{"delta":{"role":"user"}}]}]"#,
    );
    assert_eq!(role.choices[0].message.role, "assistant");

    let defaults =
        base_for(r#"[{"object":"chat.completion.chunk","created":1,"model":"orig","choices":[]}]"#);
    assert_eq!(defaults.id, "");
    assert_eq!(defaults.model, "orig");
    assert_eq!(defaults.created, 1);
    assert_eq!(defaults.system_fingerprint, None);
    assert_eq!(defaults.service_tier, None);

    // an empty model in a later chunk is skipped in favour of the original model
    let blank_model = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"orig","choices":[]},{"model":"","choices":[]}]"#,
    );
    assert_eq!(blank_model.model, "orig");
}

fn base_needs_first_fields() {
    let no_object =
        StreamedChunks::new(chunks(r#"[{"created":1,"model":"orig","choices":[]}]"#)).unwrap();
    assert_eq!(
        build_base_response(&no_object),
        Err(StreamBaseAssemblyError::MissingObject)
    );

    let no_created = StreamedChunks::new(chunks(
        r#"[{"object":"chat.completion.chunk","model":"orig","choices":[]}]"#,
    ))
    .unwrap();
    assert_eq!(
        build_base_response(&no_created),
        Err(StreamBaseAssemblyError::MissingCreated)
    );

    let no_model = StreamedChunks::new(chunks(
        r#"[{"object":"chat.completion.chunk","created":1,"choices":[]}]"#,
    ))
    .unwrap();
    assert_eq!(
        build_base_response(&no_model),
        Err(StreamBaseAssemblyError::MissingModel)
    );
}

fn finish_and_hidden() {
    finish_reason_boundaries();
    hidden_params_boundaries();
}

fn finish_reason_boundaries() {
    // the last non-None first-choice finish reason is mapped like Choices does, keeping the raw value
    let response = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"m","choices":[{"finish_reason":"length"}],"_hidden_params":{"a":1}},
            {"created":1,"model":"m","choices":[{"finish_reason":null}]},
            {"created":1,"model":"m","choices":[{"finish_reason":""}],"_hidden_params":{"b":2}}]"#,
    );
    assert_eq!(response.choices[0].finish_reason, "stop");
    assert_eq!(
        response.choices[0].provider_specific_fields,
        Some(ChoiceProviderFields {
            native_finish_reason: String::new(),
        })
    );
    assert_eq!(response.hidden_params, object(r#"{"b":2}"#));

    // an alias keeps its provider value; a reason the map knows has no provider field
    let alias = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"m","choices":[{"finish_reason":"end_turn"}]}]"#,
    );
    assert_eq!(alias.choices[0].finish_reason, "stop");
    assert_eq!(
        alias.choices[0].provider_specific_fields,
        Some(ChoiceProviderFields {
            native_finish_reason: "end_turn".to_owned(),
        })
    );

    let known = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"m","choices":[{"finish_reason":"length"}]}]"#,
    );
    assert_eq!(known.choices[0].finish_reason, "length");
    assert_eq!(known.choices[0].provider_specific_fields, None);

    let stopped = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"m","choices":[{"finish_reason":null}]}]"#,
    );
    assert_eq!(stopped.choices[0].finish_reason, "stop");
    assert_eq!(stopped.choices[0].provider_specific_fields, None);
    assert!(stopped.hidden_params.is_empty());
}

fn hidden_params_boundaries() {
    provider_model_boundaries();
    hidden_update_boundaries();
}

fn provider_model_boundaries() {
    // the first hidden string differing from the original model wins
    let provider = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"orig","_hidden_params":{"provider_response_model":"real-1"}},
            {"created":1,"model":"orig","_hidden_params":{"provider_response_model":"real-2","last":true}}]"#,
    );
    assert_eq!(
        provider.hidden_params.get("provider_response_model"),
        Some(&JsonValue::String("real-1".to_owned()))
    );
    assert_eq!(
        provider.hidden_params.get("last"),
        Some(&JsonValue::Bool(true))
    );

    // when every candidate equals the original model the first one is kept
    let same = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"orig","_hidden_params":{"provider_response_model":"orig"}},
            {"created":1,"model":"orig","_hidden_params":{"provider_response_model":""}}]"#,
    );
    assert_eq!(
        same.hidden_params.get("provider_response_model"),
        Some(&JsonValue::String("orig".to_owned()))
    );

    // a later candidate equal to the original model never wins over an earlier differing one
    let mixed = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"orig","_hidden_params":{"provider_response_model":"real-1"}},
            {"created":1,"model":"orig","_hidden_params":{"provider_response_model":"orig"}}]"#,
    );
    assert_eq!(
        mixed.hidden_params.get("provider_response_model"),
        Some(&JsonValue::String("real-1".to_owned()))
    );

    // the last chunk's hidden map is copied whole; only the overlay is skipped for an empty value
    let absent = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"m","_hidden_params":{"provider_response_model":""}}]"#,
    );
    assert_eq!(
        absent.hidden_params,
        object(r#"{"provider_response_model":""}"#)
    );
}

fn hidden_update_boundaries() {
    let mut updated = base_for(
        r#"[{"object":"chat.completion.chunk","created":1,"model":"m","_hidden_params":{"keep":1}}]"#,
    );
    apply_hidden_params(&mut updated, None);
    assert_eq!(updated.hidden_params, object(r#"{"keep":1}"#));
    let replacement = chunk(r#"{"_hidden_params":{"replaced":true}}"#);
    apply_hidden_params(&mut updated, Some(&replacement));
    assert_eq!(updated.hidden_params, object(r#"{"replaced":true}"#));
}

struct FixtureResolver {
    provider: Option<ProviderId>,
    calls: AtomicUsize,
}

impl ProviderResolution for FixtureResolver {
    fn provider_for_model(&self, model: &str) -> Result<ProviderId, ResolutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.provider
            .ok_or_else(|| ResolutionError::MissingProvider {
                model: model.to_owned(),
            })
    }
}

struct FixtureSelector {
    tag: Option<ChatConfigTag>,
    fail: bool,
    calls: AtomicUsize,
}

impl ChatConfigSelection for FixtureSelector {
    fn select_chat_config(
        &self,
        _model: &str,
        provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(ChatConfigError::MissingJsonRecord { provider });
        }
        Ok(self.tag)
    }
}

struct RecordingProjection {
    fail: bool,
    calls: Mutex<Vec<ChatConfigTag>>,
}

impl ProviderMetadataProjection for RecordingProjection {
    fn project(
        &self,
        tag: ChatConfigTag,
        response: &mut BaseResponse,
        _chunks: &[Chunk],
    ) -> Result<(), ProjectionError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(tag);
        }
        response.hidden_params.insert(
            "projected".to_owned(),
            JsonValue::String(tag.class_name().to_owned()),
        );
        if self.fail {
            return Err(ProjectionError {
                reason: "fixture projection failure".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum StageFailure {
    Selector,
    Projection,
}

struct Harness {
    redactor: Redactor,
    resolver: FixtureResolver,
    selector: FixtureSelector,
    projection: RecordingProjection,
}

impl Harness {
    fn new(tag: Option<ChatConfigTag>, failure: Option<StageFailure>) -> Self {
        let selector_fails = matches!(failure, Some(StageFailure::Selector));
        let projection_fails = matches!(failure, Some(StageFailure::Projection));
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
            resolver: FixtureResolver {
                provider: Some(ProviderId::Anthropic),
                calls: AtomicUsize::new(0),
            },
            selector: FixtureSelector {
                tag,
                fail: selector_fails,
                calls: AtomicUsize::new(0),
            },
            projection: RecordingProjection {
                fail: projection_fails,
                calls: Mutex::new(Vec::new()),
            },
        }
    }

    fn failing_resolver(tag: Option<ChatConfigTag>, failure: Option<StageFailure>) -> Self {
        let mut harness = Self::new(tag, failure);
        harness.resolver.provider = None;
        harness
    }

    fn run(&self, response: &mut BaseResponse, chunks: &[Chunk], custom: Option<&str>) {
        let context = ProviderMetadataContext {
            custom_llm_provider: custom,
            redactor: &self.redactor,
            resolver: &self.resolver,
            selector: &self.selector,
            projection: Some(&self.projection),
        };
        apply_provider_metadata(response, chunks, &context);
    }

    fn resolver_calls(&self) -> usize {
        self.resolver.calls.load(Ordering::SeqCst)
    }

    fn selector_calls(&self) -> usize {
        self.selector.calls.load(Ordering::SeqCst)
    }

    fn projected(&self) -> Vec<ChatConfigTag> {
        self.projection
            .calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

fn view_response(model: &str, hidden_params: JsonObject) -> BaseResponse {
    BaseResponse {
        id: "fixture".to_owned(),
        object: "chat.completion".to_owned(),
        created: 1,
        model: model.to_owned(),
        system_fingerprint: None,
        service_tier: None,
        choices: Vec::new(),
        usage: Usage::default(),
        hidden_params,
    }
}

fn assembled_provider_metadata() {
    assembled_noops();
    assembled_success();
    assembled_failures();
    assembled_redaction();
}

fn assembled_noops() {
    let one_chunk = chunks(r#"[{"choices":[]}]"#);
    let mut unchanged = view_response("gpt-4o", object(r#"{"start":1}"#));

    let empty = Harness::new(Some(ChatConfigTag::Anthropic), None);
    empty.run(&mut unchanged, &[], None);
    assert_eq!(empty.resolver_calls(), 0);
    assert_eq!(empty.selector_calls(), 0);
    assert!(empty.projected().is_empty());
    assert_eq!(unchanged.hidden_params, object(r#"{"start":1}"#));

    let empty_model = Harness::new(Some(ChatConfigTag::Anthropic), None);
    let mut blank = view_response("", object("{}"));
    empty_model.run(&mut blank, &one_chunk, None);
    assert_eq!(empty_model.resolver_calls(), 0);
}

fn assembled_success() {
    let one_chunk = chunks(r#"[{"choices":[]}]"#);

    // a known logging provider skips the resolver and reaches the projection with its tag
    let custom = Harness::new(Some(ChatConfigTag::Anthropic), None);
    let mut projected = view_response("claude", object("{}"));
    custom.run(&mut projected, &one_chunk, Some("anthropic"));
    assert_eq!(custom.resolver_calls(), 0);
    assert_eq!(custom.selector_calls(), 1);
    assert_eq!(custom.projected(), vec![ChatConfigTag::Anthropic]);
    assert_eq!(
        projected.hidden_params.get("projected"),
        Some(&JsonValue::String("AnthropicConfig".to_owned()))
    );

    // an empty logging provider is falsy and falls through to resolution
    let resolved = Harness::new(Some(ChatConfigTag::Anthropic), None);
    let mut via_resolver = view_response("claude", object("{}"));
    resolved.run(&mut via_resolver, &one_chunk, Some(""));
    assert_eq!(resolved.resolver_calls(), 1);
    assert_eq!(resolved.projected(), vec![ChatConfigTag::Anthropic]);
}

fn assembled_failures() {
    let one_chunk = chunks(r#"[{"choices":[]}]"#);

    // an unknown provider slug fails before selection
    let unknown = Harness::new(Some(ChatConfigTag::Anthropic), None);
    let mut anonymous = view_response("claude", object(r#"{"start":1}"#));
    let unknown_logs =
        captured_logs(|| unknown.run(&mut anonymous, &one_chunk, Some("not-a-provider")));
    assert_eq!(unknown.resolver_calls(), 0);
    assert_eq!(unknown.selector_calls(), 0);
    assert!(unknown.projected().is_empty());
    assert_eq!(anonymous.hidden_params, object(r#"{"start":1}"#));
    assert_logged(&unknown_logs, "not-a-provider");

    // a resolver failure is swallowed before selection
    let unresolved = Harness::failing_resolver(Some(ChatConfigTag::Anthropic), None);
    let mut unresolved_response = view_response("m", object("{}"));
    let unresolved_logs =
        captured_logs(|| unresolved.run(&mut unresolved_response, &one_chunk, None));
    assert_eq!(unresolved.resolver_calls(), 1);
    assert_eq!(unresolved.selector_calls(), 0);
    assert_logged(&unresolved_logs, "resolving the provider");

    // a selection failure or a missing config stops before the projection
    let failing = Harness::new(Some(ChatConfigTag::Anthropic), Some(StageFailure::Selector));
    let mut failing_response = view_response("m", object("{}"));
    let failing_logs =
        captured_logs(|| failing.run(&mut failing_response, &one_chunk, Some("anthropic")));
    assert!(failing.projected().is_empty());
    assert_logged(&failing_logs, "selecting the chat config");

    let missing = Harness::new(None, None);
    let mut missing_response = view_response("m", object("{}"));
    let missing_logs =
        captured_logs(|| missing.run(&mut missing_response, &one_chunk, Some("anthropic")));
    assert!(missing.projected().is_empty());
    assert!(missing_logs.is_empty());

    // a projection failure is swallowed after the callback ran
    let broken = Harness::new(
        Some(ChatConfigTag::Anthropic),
        Some(StageFailure::Projection),
    );
    let mut broken_response = view_response("m", object("{}"));
    let broken_logs =
        captured_logs(|| broken.run(&mut broken_response, &one_chunk, Some("anthropic")));
    assert_eq!(broken.projected(), vec![ChatConfigTag::Anthropic]);
    assert_logged(&broken_logs, "fixture projection failure");
}

fn assembled_redaction() {
    // B187 masks a credential the model and the failure text carry
    let secret = "sk-abcdefghijklmnopqrst";
    let masked = Harness::failing_resolver(Some(ChatConfigTag::Anthropic), None);
    let mut secret_response = view_response(secret, object("{}"));
    let one_chunk = chunks(r#"[{"choices":[]}]"#);
    let logs = captured_logs(|| masked.run(&mut secret_response, &one_chunk, None));
    assert!(logs.iter().any(|line| line.contains("REDACTED")));
    assert!(
        logs.iter()
            .all(|line| !line.contains("abcdefghijklmnopqrst"))
    );
}

#[derive(Clone, Default)]
struct CapturedEvents(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for CapturedEvents {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = CapturedFields::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.extend(fields.0);
        }
    }
}

#[derive(Default)]
struct CapturedFields(Vec<String>);

impl Visit for CapturedFields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push(value.to_owned());
        }
    }
}

fn captured_logs<F>(run: F) -> Vec<String>
where
    F: FnOnce(),
{
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturedEvents(Arc::clone(&events)));
    with_default(subscriber, run);
    events
        .lock()
        .map(|captured| captured.clone())
        .unwrap_or_default()
}

fn assert_logged(logs: &[String], needle: &str) {
    assert!(logs.iter().any(|line| line.contains(needle)));
}

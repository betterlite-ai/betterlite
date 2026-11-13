#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::{RouteCatalog, RouteEntry};
use indexmap::IndexMap;
use rstest::rstest;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_translation::{BaseTranslation, StreamingScanKey, TranslationCall};
use crate::unified_dispatch::emit::{
    HttpErrorDetail, StreamHttpError, a2a_jsonrpc_error_chunk, a2a_request_id,
    emit_streaming_http_error, handle_streaming_block,
};
use crate::unified_dispatch::flags::{is_redundant_scan, resolve_streaming_flag};
use crate::unified_dispatch::registry::{
    A2A_CALL_TYPES, ResponseKind, TranslationEntry, TranslationRegistry, ensure_request_metadata,
    infer_call_type, resolve_endpoint_translation, resolve_transform_call_type,
};
use crate::unified_dispatch::stream_items::{
    chunk_carries_text, chunk_choices, chunk_has_finish_reason, chunk_has_tool_calls, held_choices,
    is_trailing_metadata_chunk, record_finish_reasons, tool_call_passthrough_chunk,
    trailing_metadata_chunks,
};
use crate::unified_dispatch::transform_chunk::{TransformChunkInput, build_transform_chunk};

fn parsed(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn text(content: &str) -> GuardrailValue {
    GuardrailValue::Text(content.to_owned())
}

fn expected_choices(item: GuardrailValue) -> Vec<GuardrailValue> {
    match item {
        GuardrailValue::Array(items) => items,
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Object(_) => Vec::new(),
    }
}

fn choice_indexes(choices: &[GuardrailValue]) -> Vec<i64> {
    let mut indexes = Vec::new();
    for choice in choices {
        let GuardrailValue::Object(fields) = choice else {
            continue;
        };
        let Some(GuardrailValue::SignedInteger(index)) = fields.get("index") else {
            continue;
        };
        indexes.push(*index);
    }
    indexes
}

fn chunk(content: &str) -> GuardrailValue {
    parsed(&format!(
        r#"{{"id":"chunk-1","created":7,"model":"gpt-4o","object":"chat.completion.chunk","choices":[{{"index":0,"delta":{{"content":"{content}","role":"assistant"}},"finish_reason":null}}]}}"#
    ))
}

fn catalog(routes: Vec<(&str, Vec<CallType>)>) -> RouteCatalog {
    RouteCatalog {
        source: "test".to_owned(),
        call_types: routes
            .iter()
            .flat_map(|(_route, call_types)| call_types.clone())
            .collect(),
        routes: routes
            .into_iter()
            .map(|(route, call_types)| RouteEntry {
                route: route.to_owned(),
                call_types,
            })
            .collect(),
    }
}

fn registry(entries: Vec<(&'static str, CallType, bool)>) -> TranslationRegistry<&'static str> {
    TranslationRegistry {
        entries: entries
            .into_iter()
            .map(
                |(translation, call_type, chat_completions)| TranslationEntry {
                    call_type,
                    translation,
                    chat_completions,
                },
            )
            .collect(),
    }
}

fn text_map(pairs: &[(i64, &str)]) -> IndexMap<i64, String> {
    pairs
        .iter()
        .map(|(index, value)| (*index, (*value).to_owned()))
        .collect()
}

fn index_map(pairs: &[(i64, i64)]) -> IndexMap<i64, i64> {
    pairs.iter().copied().collect()
}

fn reason_map(pairs: &[(i64, Option<&str>)]) -> IndexMap<i64, Option<String>> {
    pairs
        .iter()
        .map(|(index, reason)| (*index, reason.map(str::to_owned)))
        .collect()
}

enum BlockBehavior {
    Missing,
    Fixed(Vec<Vec<u8>>),
    Summary,
}

struct ProbeTranslation {
    block: BlockBehavior,
    error_items: Option<Vec<GuardrailValue>>,
}

impl BaseTranslation for ProbeTranslation {
    type Guardrail = ();
    type LogTarget = ();
    type Failure = GuardrailFailure;

    fn process_input_messages(
        &self,
        _data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(response))
    }

    fn build_block_sse_chunks(
        &self,
        _failure: &Self::Failure,
        stream_started: bool,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<Vec<u8>>> {
        match &self.block {
            BlockBehavior::Missing => None,
            BlockBehavior::Fixed(chunks) => Some(chunks.clone()),
            BlockBehavior::Summary => {
                let count = responses_so_far.map_or(0, <[GuardrailValue]>::len);
                Some(vec![
                    format!("started={stream_started};items={count}").into_bytes(),
                ])
            }
        }
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<GuardrailValue>> {
        self.error_items.as_ref().map(|items| {
            let count = responses_so_far.map_or(0, <[GuardrailValue]>::len);
            let mut echoed = items.clone();
            echoed.push(text(&format!("items={count};{failure}")));
            echoed
        })
    }
}

fn probe() -> ProbeTranslation {
    ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: None,
    }
}

// registry resolution

#[test]
fn dispatch_registry_returns_each_entry() {
    let registry = registry(vec![
        ("chat", CallType::Completion, true),
        ("embedding", CallType::Embedding, false),
    ]);
    assert_eq!(
        registry
            .get(CallType::Completion)
            .map(|entry| entry.translation),
        Some("chat")
    );
    assert_eq!(
        registry
            .get(CallType::Embedding)
            .map(|entry| entry.translation),
        Some("embedding")
    );
}

#[test]
fn dispatch_registry_later_entry_replaces_an_earlier_one() {
    let registry = registry(vec![
        ("first", CallType::Completion, true),
        ("second", CallType::Completion, false),
    ]);
    let entry = registry.get(CallType::Completion).unwrap();
    assert_eq!(entry.translation, "second");
    assert!(!entry.chat_completions);
}

#[test]
fn dispatch_registry_missing_call_type_is_none() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    assert!(registry.get(CallType::Embedding).is_none());
}

#[test]
fn dispatch_route_selects_the_first_call_type() {
    let registry = registry(vec![
        ("chat", CallType::Completion, true),
        ("embedding", CallType::Embedding, false),
    ]);
    let routes = catalog(vec![(
        "/chat/completions",
        vec![CallType::Completion, CallType::Embedding],
    )]);
    let resolved = resolve_endpoint_translation(
        &registry,
        &routes,
        Some("/chat/completions"),
        Some(&chunk("hi")),
    );
    assert_eq!(
        resolved.map(|(call_type, translation)| (call_type, *translation)),
        Some((CallType::Completion, "chat"))
    );
}

#[test]
fn dispatch_route_first_call_type_without_entry_is_none() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    let routes = catalog(vec![(
        "/completions",
        vec![CallType::TextCompletion, CallType::Completion],
    )]);
    assert!(resolve_endpoint_translation(&registry, &routes, Some("/completions"), None).is_none());
}

#[test]
fn dispatch_route_wins_over_the_item_inference() {
    let registry = registry(vec![
        ("chat", CallType::Completion, true),
        ("embedding", CallType::Embedding, false),
    ]);
    let routes = catalog(vec![("/embeddings", vec![CallType::Embedding])]);
    let resolved =
        resolve_endpoint_translation(&registry, &routes, Some("/embeddings"), Some(&chunk("hi")));
    assert_eq!(
        resolved.map(|(call_type, translation)| (call_type, *translation)),
        Some((CallType::Embedding, "embedding"))
    );
}

#[test]
fn dispatch_missing_route_infers_a_completion_from_a_chunk() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    let routes = catalog(Vec::new());
    let resolved = resolve_endpoint_translation(&registry, &routes, None, Some(&chunk("hi")));
    assert_eq!(
        resolved.map(|(call_type, _)| call_type),
        Some(CallType::Completion)
    );
}

#[test]
fn dispatch_empty_route_infers_from_the_item() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    let routes = catalog(vec![("/chat/completions", vec![CallType::Completion])]);
    let resolved = resolve_endpoint_translation(&registry, &routes, Some(""), Some(&chunk("hi")));
    assert_eq!(
        resolved.map(|(call_type, _)| call_type),
        Some(CallType::Completion)
    );
}

#[test]
fn dispatch_no_route_and_no_item_is_none() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    let routes = catalog(Vec::new());
    assert!(resolve_endpoint_translation(&registry, &routes, None, None).is_none());
}

#[test]
fn dispatch_unmapped_inferred_call_type_is_none() {
    let registry = registry(vec![("embedding", CallType::Embedding, false)]);
    let routes = catalog(Vec::new());
    assert!(resolve_endpoint_translation(&registry, &routes, None, Some(&chunk("hi"))).is_none());
}

#[test]
fn dispatch_inferred_non_chat_item_is_none() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    let routes = catalog(Vec::new());
    let frame = text("data: {}");
    assert!(resolve_endpoint_translation(&registry, &routes, None, Some(&frame)).is_none());
}

#[test]
fn dispatch_transform_call_type_needs_the_chat_flag() {
    let registry = registry(vec![("chat", CallType::Completion, false)]);
    let routes = catalog(vec![("/chat/completions", vec![CallType::Completion])]);
    assert_eq!(
        resolve_transform_call_type(&registry, &routes, Some("/chat/completions")),
        None
    );
}

#[test]
fn dispatch_transform_call_type_accepts_a_chat_entry() {
    let registry = registry(vec![("chat", CallType::Acompletion, true)]);
    let routes = catalog(vec![("/a/chat/completions", vec![CallType::Acompletion])]);
    assert_eq!(
        resolve_transform_call_type(&registry, &routes, Some("/a/chat/completions")),
        Some(CallType::Acompletion)
    );
}

#[test]
fn dispatch_transform_call_type_ignores_a_later_chat_entry() {
    let registry = registry(vec![
        ("embedding", CallType::Embedding, false),
        ("chat", CallType::Completion, true),
    ]);
    let routes = catalog(vec![(
        "/mixed",
        vec![CallType::Embedding, CallType::Completion],
    )]);
    assert_eq!(
        resolve_transform_call_type(&registry, &routes, Some("/mixed")),
        None
    );
}

#[test]
fn dispatch_transform_call_type_needs_a_route() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    let routes = catalog(vec![("/chat/completions", vec![CallType::Completion])]);
    assert_eq!(resolve_transform_call_type(&registry, &routes, None), None);
    assert_eq!(
        resolve_transform_call_type(&registry, &routes, Some("")),
        None
    );
}

#[test]
fn dispatch_transform_call_type_unmapped_route_is_none() {
    let registry = registry(vec![("chat", CallType::Completion, true)]);
    let routes = catalog(Vec::new());
    assert_eq!(
        resolve_transform_call_type(&registry, &routes, Some("/unknown")),
        None
    );
}

#[rstest]
#[case::chat(ResponseKind::ChatCompletion, Some(CallType::Completion))]
#[case::embedding(ResponseKind::Embedding, Some(CallType::Embedding))]
#[case::transcription(ResponseKind::Transcription, Some(CallType::Transcription))]
#[case::speech(ResponseKind::Speech, Some(CallType::Speech))]
#[case::rerank(ResponseKind::Rerank, Some(CallType::Rerank))]
#[case::image(ResponseKind::Image, Some(CallType::ImageGeneration))]
#[case::text_completion(ResponseKind::TextCompletion, Some(CallType::TextCompletion))]
#[case::send_message(ResponseKind::SendMessage, Some(CallType::SendMessage))]
#[case::other(ResponseKind::Other, None)]
fn dispatch_infer_call_type_maps_every_kind(
    #[case] kind: ResponseKind,
    #[case] expected: Option<CallType>,
) {
    assert_eq!(infer_call_type(kind), expected);
}

#[rstest]
#[case::chat_chunk(
    parsed(r#"{"object": "chat.completion.chunk", "choices": []}"#),
    ResponseKind::ChatCompletion
)]
#[case::model_response(parsed(r#"{"object": "chat.completion"}"#), ResponseKind::Other)]
#[case::plain_object(parsed(r#"{"id": "1"}"#), ResponseKind::Other)]
#[case::string(text("data: {}"), ResponseKind::Other)]
#[case::array(
    parsed(r#"[{"object": "chat.completion.chunk"}]"#),
    ResponseKind::Other
)]
#[case::null(GuardrailValue::Null, ResponseKind::Other)]
fn dispatch_of_stream_item_kinds(#[case] item: GuardrailValue, #[case] kind: ResponseKind) {
    assert_eq!(ResponseKind::of_stream_item(&item), kind);
}

#[test]
fn dispatch_ensure_request_metadata_sets_an_absent_key() {
    let mut data = BTreeMap::new();
    let user = BTreeMap::from([("user_api_key_user_id".to_owned(), text("u-1"))]);
    ensure_request_metadata(&mut data, user.clone());
    assert_eq!(
        data.get("litellm_metadata"),
        Some(&GuardrailValue::Object(user))
    );
}

#[test]
fn dispatch_ensure_request_metadata_keeps_an_existing_key() {
    let existing = GuardrailValue::Object(BTreeMap::new());
    let mut data = BTreeMap::from([("litellm_metadata".to_owned(), existing.clone())]);
    let user = BTreeMap::from([("user_api_key_user_id".to_owned(), text("u-1"))]);
    ensure_request_metadata(&mut data, user);
    assert_eq!(data.get("litellm_metadata"), Some(&existing));
}

#[test]
fn dispatch_ensure_request_metadata_skips_empty_user_metadata() {
    let mut data = BTreeMap::new();
    ensure_request_metadata(&mut data, BTreeMap::new());
    assert!(data.is_empty());
}

#[test]
fn dispatch_a2a_call_types_hold_asend_and_send() {
    assert_eq!(
        A2A_CALL_TYPES,
        [CallType::AsendMessage, CallType::SendMessage]
    );
}

// emit

#[test]
fn emit_block_returns_the_translation_chunks_in_order() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Fixed(vec![b"first".to_vec(), b"second".to_vec()]),
        error_items: None,
    };
    let failure = GuardrailFailure::Other("blocked".to_owned());
    let chunks = handle_streaming_block(&translation, failure, true, &[]).unwrap();
    assert_eq!(chunks, vec![b"first".to_vec(), b"second".to_vec()]);
}

#[test]
fn emit_block_starts_a_new_message_when_the_stream_did_not_start() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Summary,
        error_items: None,
    };
    let chunks = handle_streaming_block(
        &translation,
        GuardrailFailure::Other("blocked".to_owned()),
        false,
        &[chunk("hi")],
    )
    .unwrap();
    assert_eq!(chunks, vec![b"started=false;items=1".to_vec()]);
}

#[test]
fn emit_block_gives_back_the_failure_when_the_translation_has_none() {
    let failure = GuardrailFailure::Other("blocked".to_owned());
    let error = handle_streaming_block(&probe(), failure.clone(), true, &[]).unwrap_err();
    assert_eq!(error, failure);
}

#[test]
fn emit_http_error_returns_the_a2a_chunk_before_the_stream_starts() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: Some(vec![text("translation")]),
    };
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let request_data = BTreeMap::new();
    let items = emit_streaming_http_error(
        Some(CallType::SendMessage),
        error,
        &[parsed(r#"{"id": 9}"#)],
        &request_data,
        Some(&translation),
        false,
        None,
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    let expected = parsed(
        r#"{"jsonrpc": "2.0", "id": 9, "error": {"code": -32603, "message": "blocked", "data": {}}}"#,
    );
    assert_eq!(items.first(), Some(&expected));
}

#[test]
fn emit_http_error_uses_the_yielded_items_not_the_so_far_items() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: Some(vec![text("frame")]),
    };
    let error = StreamHttpError {
        status_code: 422,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let yielded = [chunk("sent")];
    let items = emit_streaming_http_error(
        Some(CallType::Completion),
        error,
        &[chunk("a"), chunk("b"), chunk("c")],
        &BTreeMap::new(),
        Some(&translation),
        true,
        Some(&yielded),
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items.get(1), Some(&text("items=1;blocked")));
}

#[test]
fn emit_http_error_not_started_gives_back_the_error() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: Some(vec![text("frame")]),
    };
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let result = emit_streaming_http_error(
        Some(CallType::Completion),
        error.clone(),
        &[],
        &BTreeMap::new(),
        Some(&translation),
        false,
        None,
    );
    assert_eq!(result, Err(error));
}

#[test]
fn emit_http_error_without_a_translation_gives_back_the_error() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let result = emit_streaming_http_error(
        Some(CallType::Completion),
        error.clone(),
        &[],
        &BTreeMap::new(),
        None::<&ProbeTranslation>,
        true,
        None,
    );
    assert_eq!(result, Err(error));
}

#[test]
fn emit_http_error_without_error_items_gives_back_the_error() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let result = emit_streaming_http_error(
        Some(CallType::Completion),
        error.clone(),
        &[],
        &BTreeMap::new(),
        Some(&probe()),
        true,
        None,
    );
    assert_eq!(result, Err(error));
}

#[test]
fn emit_http_error_hands_a_text_detail_to_the_translation() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: Some(Vec::new()),
    };
    let error = StreamHttpError {
        status_code: 403,
        detail: HttpErrorDetail::Text("denied".to_owned()),
    };
    let items = emit_streaming_http_error(
        Some(CallType::Completion),
        error,
        &[],
        &BTreeMap::new(),
        Some(&translation),
        true,
        None,
    )
    .unwrap();
    assert_eq!(items, vec![text("items=0;denied")]);
}

#[test]
fn emit_http_error_hands_an_object_detail_to_the_translation() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: Some(Vec::new()),
    };
    let detail = BTreeMap::from([("message".to_owned(), text("denied"))]);
    let error = StreamHttpError {
        status_code: 403,
        detail: HttpErrorDetail::Object(detail),
    };
    let items = emit_streaming_http_error(
        Some(CallType::Completion),
        error,
        &[],
        &BTreeMap::new(),
        Some(&translation),
        true,
        None,
    )
    .unwrap();
    assert_eq!(items, vec![text(r#"items=0;{"message":"denied"}"#)]);
}

// a2a

#[test]
fn a2a_request_id_reads_an_object_item() {
    let responses = [parsed(r#"{"id": 42, "jsonrpc": "2.0"}"#)];
    assert_eq!(
        a2a_request_id(&responses, &BTreeMap::new()),
        GuardrailValue::SignedInteger(42)
    );
}

#[test]
fn a2a_request_id_reads_a_json_string_item() {
    let responses = [text(" {\"id\": \"req-1\"} ")];
    assert_eq!(a2a_request_id(&responses, &BTreeMap::new()), text("req-1"));
}

#[test]
fn a2a_request_id_null_in_an_object_item_ends_the_search() {
    let responses = [parsed(r#"{"id": null}"#), parsed(r#"{"id": 7}"#)];
    assert_eq!(
        a2a_request_id(&responses, &BTreeMap::new()),
        GuardrailValue::Null
    );
}

#[test]
fn a2a_request_id_skips_strings_without_an_id() {
    let responses = [text(r#"{"jsonrpc": "2.0"}"#), parsed(r#"{"id": 7}"#)];
    assert_eq!(
        a2a_request_id(&responses, &BTreeMap::new()),
        GuardrailValue::SignedInteger(7)
    );
}

#[test]
fn a2a_request_id_skips_unparsable_strings() {
    let responses = [
        text("not json"),
        text("[1, 2]"),
        text("null"),
        parsed(r#"{"id": 7}"#),
    ];
    assert_eq!(
        a2a_request_id(&responses, &BTreeMap::new()),
        GuardrailValue::SignedInteger(7)
    );
}

#[test]
fn a2a_request_id_reads_the_body() {
    let request_data = BTreeMap::from([("body".to_owned(), parsed(r#"{"id": "body-id"}"#))]);
    assert_eq!(a2a_request_id(&[], &request_data), text("body-id"));
}

#[test]
fn a2a_request_id_empty_body_reads_the_data() {
    let request_data = BTreeMap::from([
        ("body".to_owned(), parsed("{}")),
        ("data".to_owned(), parsed(r#"{"id": "data-id"}"#)),
    ]);
    assert_eq!(a2a_request_id(&[], &request_data), text("data-id"));
}

#[test]
fn a2a_request_id_without_a_body_id_is_null() {
    let request_data = BTreeMap::from([("body".to_owned(), parsed(r#"{"jsonrpc": "2.0"}"#))]);
    assert_eq!(a2a_request_id(&[], &request_data), GuardrailValue::Null);
    assert_eq!(a2a_request_id(&[], &BTreeMap::new()), GuardrailValue::Null);
}

#[test]
fn a2a_error_chunk_takes_the_error_key_as_the_message() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Object(BTreeMap::from([
            ("error".to_owned(), text("boom")),
            ("message".to_owned(), text("ignored")),
            ("code".to_owned(), GuardrailValue::SignedInteger(5)),
        ])),
    };
    let chunk = a2a_jsonrpc_error_chunk(&error, GuardrailValue::Null);
    assert_eq!(
        chunk,
        parsed(
            r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": "boom", "data": {"code": 5}}}"#
        )
    );
}

#[test]
fn a2a_error_chunk_falls_back_to_the_message_key() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Object(BTreeMap::from([("message".to_owned(), text("denied"))])),
    };
    let chunk = a2a_jsonrpc_error_chunk(&error, text("req"));
    assert_eq!(
        chunk,
        parsed(
            r#"{"jsonrpc": "2.0", "id": "req", "error": {"code": -32603, "message": "denied", "data": {}}}"#
        )
    );
}

#[test]
fn a2a_error_chunk_writes_an_object_without_keys_as_json_text() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Object(BTreeMap::from([("reason".to_owned(), text("denied"))])),
    };
    let chunk = a2a_jsonrpc_error_chunk(&error, GuardrailValue::Null);
    assert_eq!(
        chunk,
        parsed(
            r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": "{\"reason\":\"denied\"}", "data": {"reason": "denied"}}}"#
        )
    );
}

#[test]
fn a2a_error_chunk_wraps_a_text_detail() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("denied".to_owned()),
    };
    let chunk = a2a_jsonrpc_error_chunk(&error, GuardrailValue::Null);
    assert_eq!(
        chunk,
        parsed(
            r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": "denied", "data": {}}}"#
        )
    );
}

// transform chunk

#[test]
fn transform_chunk_emits_only_the_new_text() {
    let mutated = text_map(&[(0, "Hello world")]);
    let mut emitted = text_map(&[(0, "Hello")]);
    let mut held = IndexMap::new();
    let reference = chunk("Hello");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": " world", "role": null, "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
    assert_eq!(emitted.get(&0).map(String::as_str), Some("Hello world"));
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_keeps_the_holdback_before_the_final_round() {
    let mutated = text_map(&[(0, "Hello world")]);
    let mut emitted = IndexMap::new();
    let holdback = index_map(&[(0, 5)]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "Hello ", "role": "assistant", "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
    assert_eq!(held.get(&0), Some(&5));
    assert_eq!(emitted.get(&0).map(String::as_str), Some("Hello "));
}

#[test]
fn transform_chunk_forces_the_holdback_to_zero_on_the_final_round() {
    let mutated = text_map(&[(0, "Hello world")]);
    let mut emitted = IndexMap::new();
    let holdback = index_map(&[(0, 5)]);
    let mut held = index_map(&[(0, 5)]);
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &reason_map(&[(0, Some("stop"))]),
        held_chars_per_choice: &mut held,
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "Hello world", "role": "assistant", "tool_calls": null}, "finish_reason": "stop"}]}"#
        )
    );
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_missing_holdback_counts_as_zero() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_negative_holdback_counts_as_zero() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &index_map(&[(0, -3)]),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_counts_characters_not_bytes() {
    let mutated = text_map(&[(0, "h\u{e9}llo w\u{f6}rld")]);
    let mut emitted = text_map(&[(0, "h\u{e9}llo")]);
    let holdback = index_map(&[(0, 5)]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": " ", "role": null, "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
    assert_eq!(held.get(&0), Some(&5));
    assert_eq!(emitted.get(&0).map(String::as_str), Some("h\u{e9}llo "));
}

#[test]
fn transform_chunk_holds_everything_without_a_chunk() {
    let mutated = text_map(&[(0, "Hi")]);
    let mut emitted = IndexMap::new();
    let holdback = index_map(&[(0, 5)]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap();
    assert!(output.is_none());
    assert_eq!(held.get(&0), Some(&2));
    assert!(emitted.is_empty());
}

#[test]
fn transform_chunk_underflow_returns_the_400() {
    let mutated = text_map(&[(2, "Hx")]);
    let mut emitted = text_map(&[(2, "Hello")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let error = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap_err();
    assert_eq!(error.status_code, 400);
    let expected = "Guardrail streaming transform for choice 2 is not a forward extension of the 5 chars already streamed to the client (it is shorter than, or rewrites, the emitted prefix); emitted bytes cannot be retracted. Withhold recent output via stream_holdback_chars before rewriting it.";
    assert_eq!(
        error.detail,
        HttpErrorDetail::Object(BTreeMap::from([
            ("error".to_owned(), text("stream_transform_underflow")),
            ("message".to_owned(), text(expected)),
        ]))
    );
    assert_eq!(emitted.len(), 1);
}

#[test]
fn transform_chunk_underflow_fails_the_whole_round() {
    let mutated = text_map(&[(0, "ab"), (1, "xy")]);
    let mut emitted = text_map(&[(0, "a"), (1, "zz")]);
    let mut held = index_map(&[(0, 0), (1, 0)]);
    let reference = chunk("");
    let error = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &index_map(&[(0, 1), (1, 1)]),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap_err();
    assert_eq!(error.status_code, 400);
    assert_eq!(emitted.len(), 2);
    assert_eq!(emitted.get(&0).map(String::as_str), Some("a"));
    assert_eq!(emitted.get(&1).map(String::as_str), Some("zz"));
}

#[test]
fn transform_chunk_terminator_carries_the_finish_reasons() {
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("stop")), (1, None)]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "", "role": null, "tool_calls": null}, "finish_reason": "stop"}]}"#
        )
    );
}

#[test]
fn transform_chunk_terminator_needs_the_final_round() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("stop"))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: false,
    })
    .unwrap();
    assert!(output.is_none());
}

#[test]
fn transform_chunk_terminator_without_reasons_is_none() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, None), (1, None)]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap();
    assert!(output.is_none());
}

#[test]
fn transform_chunk_maps_an_anthropic_finish_reason() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("end_turn"))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "", "role": null, "tool_calls": null}, "finish_reason": "stop"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn transform_chunk_finish_reason_only_counts_on_the_final_round() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = text_map(&[(0, "a")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some("stop"))]),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "b", "role": null, "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn transform_chunk_role_is_assistant_only_on_the_first_delta() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    let first = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": "ab", "role": "assistant", "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&first), expected_choices(expected));
    let next = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &text_map(&[(0, "abc")]),
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let next_expected = parsed(
        r#"[{"index": 0, "delta": {"content": "c", "role": null, "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&next), expected_choices(next_expected));
}

#[test]
fn transform_chunk_skips_a_choice_with_nothing_to_say() {
    let mutated = text_map(&[(0, "ab"), (1, "cd")]);
    let mut emitted = text_map(&[(0, "ab")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    let expected = parsed(
        r#"[{"index": 1, "delta": {"content": "cd", "role": "assistant", "tool_calls": null}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
    assert_eq!(emitted.get(&0).map(String::as_str), Some("ab"));
    assert_eq!(emitted.get(&1).map(String::as_str), Some("cd"));
}

#[test]
fn transform_chunk_returns_none_when_no_choice_is_left() {
    let mutated = text_map(&[(0, "ab")]);
    let mut emitted = text_map(&[(0, "ab")]);
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap();
    assert!(output.is_none());
    assert_eq!(emitted.get(&0).map(String::as_str), Some("ab"));
    assert_eq!(held.get(&0), Some(&0));
}

#[test]
fn transform_chunk_inserts_an_empty_emitted_text_for_a_skipped_choice() {
    let mutated = text_map(&[(0, ""), (1, "cd")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(emitted.get(&0).map(String::as_str), Some(""));
    assert_eq!(emitted.get(&1).map(String::as_str), Some("cd"));
}

#[test]
fn transform_chunk_keeps_the_mutated_choice_order() {
    let mutated = text_map(&[(2, "bb"), (0, "aa")]);
    let mut emitted = IndexMap::new();
    let mut held = IndexMap::new();
    let reference = chunk("");
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut held,
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(choice_indexes(chunk_choices(&output)), vec![2, 0]);
}

#[test]
fn transform_chunk_missing_reference_fields_are_null() {
    let reference = GuardrailValue::Object(BTreeMap::new());
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &reference,
        mutated_text_per_choice: &text_map(&[(0, "a")]),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &IndexMap::new(),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: false,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": null, "created": null, "model": null, "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "a", "role": "assistant", "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
}

#[test]
fn transform_chunk_empty_finish_reason_is_no_reason_in_the_chunk() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &IndexMap::new(),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some(""))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "", "role": null, "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
}

#[test]
fn transform_chunk_empty_reason_keeps_a_choice_with_no_delta() {
    let output = build_transform_chunk(TransformChunkInput {
        reference_chunk: &chunk(""),
        mutated_text_per_choice: &text_map(&[(0, "")]),
        emitted_text_per_choice: &mut IndexMap::new(),
        holdback_per_choice: &IndexMap::new(),
        finish_reason_per_choice: &reason_map(&[(0, Some(""))]),
        held_chars_per_choice: &mut IndexMap::new(),
        is_final: true,
    })
    .unwrap()
    .unwrap();
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "", "role": "assistant", "tool_calls": null}, "finish_reason": null}]}"#
        )
    );
}

// stream items

#[test]
fn stream_item_choices_are_read_from_the_array() {
    let item = chunk("hi");
    assert_eq!(chunk_choices(&item).len(), 1);
}

#[rstest]
#[case::text(text("data: {}"))]
#[case::null(GuardrailValue::Null)]
#[case::array(GuardrailValue::Array(Vec::new()))]
#[case::no_choices(parsed(r#"{"id": "1"}"#))]
#[case::choices_not_an_array(parsed(r#"{"choices": "none"}"#))]
fn stream_item_choices_are_empty(#[case] item: GuardrailValue) {
    assert!(chunk_choices(&item).is_empty());
}

#[rstest]
#[case::text(parsed(r#"{"choices": [{"delta": {"content": "hi"}}]}"#), true)]
#[case::empty(parsed(r#"{"choices": [{"delta": {"content": ""}}]}"#), false)]
#[case::null(parsed(r#"{"choices": [{"delta": {"content": null}}]}"#), false)]
#[case::missing(parsed(r#"{"choices": [{"delta": {}}]}"#), false)]
#[case::no_delta(parsed(r#"{"choices": [{"index": 0}]}"#), false)]
#[case::other_choice(
    parsed(r#"{"choices": [{"delta": {"content": ""}}, {"delta": {"content": "x"}}]}"#),
    true
)]
fn stream_item_carries_text(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(chunk_carries_text(&item), expected);
}

#[rstest]
#[case::calls(
    parsed(r#"{"choices": [{"delta": {"tool_calls": [{"id": "c"}]}}]}"#),
    true
)]
#[case::empty(parsed(r#"{"choices": [{"delta": {"tool_calls": []}}]}"#), false)]
#[case::null(parsed(r#"{"choices": [{"delta": {"tool_calls": null}}]}"#), false)]
#[case::missing(parsed(r#"{"choices": [{"delta": {"content": "hi"}}]}"#), false)]
#[case::no_choices(text("frame"), false)]
fn stream_item_has_tool_calls(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(chunk_has_tool_calls(&item), expected);
}

#[rstest]
#[case::reason(parsed(r#"{"choices": [{"finish_reason": "stop"}]}"#), true)]
#[case::null(parsed(r#"{"choices": [{"finish_reason": null}]}"#), false)]
#[case::empty(parsed(r#"{"choices": [{"finish_reason": ""}]}"#), false)]
#[case::second_choice(
    parsed(r#"{"choices": [{"finish_reason": null}, {"finish_reason": "length"}]}"#),
    true
)]
fn stream_item_has_finish_reason(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(chunk_has_finish_reason(&item), expected);
}

#[rstest]
#[case::no_choices(parsed(r#"{"id": "1"}"#), true)]
#[case::frame(text("frame"), true)]
#[case::usage_only(
    parsed(r#"{"choices": [{"delta": {"content": ""}}], "usage": {"total_tokens": 3}}"#),
    true
)]
#[case::usage_with_text(
    parsed(r#"{"choices": [{"delta": {"content": "hi"}}], "usage": {"total_tokens": 3}}"#),
    false
)]
#[case::usage_with_finish(
    parsed(
        r#"{"choices": [{"delta": {"content": ""}, "finish_reason": "stop"}], "usage": {"total_tokens": 3}}"#
    ),
    false
)]
#[case::no_usage(parsed(r#"{"choices": [{"delta": {"content": ""}}]}"#), false)]
#[case::null_usage(
    parsed(r#"{"choices": [{"delta": {"content": ""}}], "usage": null}"#),
    false
)]
fn stream_item_trailing_metadata(#[case] item: GuardrailValue, #[case] expected: bool) {
    assert_eq!(is_trailing_metadata_chunk(&item), expected);
}

#[test]
fn stream_item_trailing_metadata_chunks_keep_the_order() {
    let items = [
        parsed(r#"{"id": "a"}"#),
        chunk("hi"),
        parsed(r#"{"choices": [], "usage": {"total_tokens": 3}}"#),
        parsed(r#"{"id": "b"}"#),
    ];
    let selected: Vec<&GuardrailValue> = trailing_metadata_chunks(&items);
    assert_eq!(selected.len(), 3);
    assert_eq!(selected.first().copied(), items.first());
    assert_eq!(selected.get(2).copied(), items.get(3));
}

#[test]
fn stream_item_records_finish_reasons_by_choice_index() {
    let item = parsed(
        r#"{"choices": [{"index": 0, "finish_reason": "stop"}, {"index": 1, "finish_reason": null}, {"index": 2, "finish_reason": "end_turn"}]}"#,
    );
    let mut reasons = IndexMap::new();
    record_finish_reasons(&item, &mut reasons);
    assert_eq!(reasons.get(&0), Some(&Some("stop".to_owned())));
    assert_eq!(reasons.get(&1), None);
    assert_eq!(reasons.get(&2), Some(&Some("stop".to_owned())));
}

#[test]
fn stream_item_records_a_null_index_as_choice_zero() {
    let item = parsed(r#"{"choices": [{"index": null, "finish_reason": "length"}]}"#);
    let mut reasons = IndexMap::new();
    record_finish_reasons(&item, &mut reasons);
    assert_eq!(reasons.get(&0), Some(&Some("length".to_owned())));
}

#[test]
fn stream_item_records_skip_an_empty_reason() {
    let item = parsed(r#"{"choices": [{"index": 0, "finish_reason": ""}]}"#);
    let mut reasons = IndexMap::new();
    record_finish_reasons(&item, &mut reasons);
    assert!(reasons.is_empty());
}

#[test]
fn stream_item_tool_call_passthrough_strips_the_text() {
    let item = parsed(
        r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": "secret", "role": "assistant", "tool_calls": [{"id": "call_1", "type": "function"}]}, "finish_reason": null}]}"#,
    );
    let output = tool_call_passthrough_chunk(&item, None, &BTreeSet::new());
    assert_eq!(
        output,
        parsed(
            r#"{"id": "chunk-1", "created": 7, "model": "gpt-4o", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {"content": null, "role": "assistant", "tool_calls": [{"id": "call_1", "type": "function"}]}, "finish_reason": null}]}"#
        )
    );
}

#[test]
fn stream_item_tool_call_passthrough_defers_a_text_choice_reason() {
    let item = parsed(
        r#"{"choices": [{"index": 3, "delta": {"content": "secret", "tool_calls": [{"id": "c"}]}, "finish_reason": "end_turn"}]}"#,
    );
    let mut reasons = IndexMap::new();
    let output = tool_call_passthrough_chunk(&item, Some(&mut reasons), &BTreeSet::new());
    assert_eq!(reasons.get(&3), Some(&Some("stop".to_owned())));
    let expected = parsed(
        r#"[{"index": 3, "delta": {"content": null, "role": null, "tool_calls": [{"id": "c"}]}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_defers_a_held_choice_reason() {
    let item = parsed(
        r#"{"choices": [{"index": 1, "delta": {"tool_calls": [{"id": "c"}]}, "finish_reason": "tool_calls"}]}"#,
    );
    let mut reasons = IndexMap::new();
    let held = BTreeSet::from([1]);
    let output = tool_call_passthrough_chunk(&item, Some(&mut reasons), &held);
    assert_eq!(reasons.get(&1), Some(&Some("tool_calls".to_owned())));
    let expected = parsed(
        r#"[{"index": 1, "delta": {"content": null, "role": null, "tool_calls": [{"id": "c"}]}, "finish_reason": null}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_keeps_other_reasons() {
    let item = parsed(
        r#"{"choices": [{"index": 0, "delta": {"tool_calls": [{"id": "c"}]}, "finish_reason": "tool_calls"}]}"#,
    );
    let mut reasons = IndexMap::new();
    let output = tool_call_passthrough_chunk(&item, Some(&mut reasons), &BTreeSet::new());
    assert!(reasons.is_empty());
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": null, "role": null, "tool_calls": [{"id": "c"}]}, "finish_reason": "tool_calls"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_without_a_map_keeps_the_reason() {
    let item = parsed(
        r#"{"choices": [{"index": 0, "delta": {"content": "secret"}, "finish_reason": "stop"}]}"#,
    );
    let output = tool_call_passthrough_chunk(&item, None, &BTreeSet::new());
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": null, "role": null, "tool_calls": null}, "finish_reason": "stop"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_tool_call_passthrough_maps_the_reason() {
    let item = parsed(r#"{"choices": [{"index": 0, "delta": {}, "finish_reason": "end_turn"}]}"#);
    let output = tool_call_passthrough_chunk(&item, None, &BTreeSet::new());
    let expected = parsed(
        r#"[{"index": 0, "delta": {"content": null, "role": null, "tool_calls": null}, "finish_reason": "stop"}]"#,
    );
    assert_eq!(chunk_choices(&output), expected_choices(expected));
}

#[test]
fn stream_item_held_choices_keep_only_positive_counts() {
    let held = index_map(&[(0, 2), (1, 0), (2, -1), (3, 5)]);
    assert_eq!(held_choices(&held), BTreeSet::from([0, 3]));
}

// flags

#[test]
fn flag_default_applies_when_nothing_is_set() {
    let default = GuardrailValue::Boolean(false);
    assert_eq!(
        resolve_streaming_flag(
            None,
            None,
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &default
        ),
        default
    );
}

#[test]
fn flag_attribute_beats_the_default() {
    let attribute = GuardrailValue::Boolean(true);
    assert_eq!(
        resolve_streaming_flag(
            Some(&attribute),
            None,
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        attribute
    );
}

#[test]
fn flag_config_beats_the_attribute() {
    let config = BTreeMap::from([(
        "streaming_end_of_stream_only".to_owned(),
        GuardrailValue::Boolean(false),
    )]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            Some(&config),
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Boolean(false)
    );
}

#[test]
fn flag_optional_params_beat_the_config() {
    let config = BTreeMap::from([(
        "streaming_sampling_rate".to_owned(),
        GuardrailValue::SignedInteger(5),
    )]);
    let optional_params = BTreeMap::from([(
        "streaming_sampling_rate".to_owned(),
        GuardrailValue::SignedInteger(9),
    )]);
    assert_eq!(
        resolve_streaming_flag(
            None,
            Some(&config),
            &optional_params,
            "streaming_sampling_rate",
            &GuardrailValue::SignedInteger(1),
        ),
        GuardrailValue::SignedInteger(9)
    );
}

#[test]
fn flag_a_present_null_in_the_config_wins() {
    let config = BTreeMap::from([(
        "streaming_end_of_stream_only".to_owned(),
        GuardrailValue::Null,
    )]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            Some(&config),
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Null
    );
}

#[test]
fn flag_a_present_null_in_the_optional_params_wins() {
    let optional_params = BTreeMap::from([(
        "streaming_end_of_stream_only".to_owned(),
        GuardrailValue::Null,
    )]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            None,
            &optional_params,
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Null
    );
}

#[test]
fn flag_config_without_the_name_keeps_the_attribute() {
    let config = BTreeMap::from([("other_key".to_owned(), GuardrailValue::Boolean(false))]);
    assert_eq!(
        resolve_streaming_flag(
            Some(&GuardrailValue::Boolean(true)),
            Some(&config),
            &BTreeMap::new(),
            "streaming_end_of_stream_only",
            &GuardrailValue::Boolean(false),
        ),
        GuardrailValue::Boolean(true)
    );
}

fn scan_key(texts: &[&str]) -> StreamingScanKey {
    StreamingScanKey {
        texts: texts.iter().map(|value| (*value).to_owned()).collect(),
        tool_calls: Vec::new(),
        stream_ended: false,
        tool_calls_in_flight: false,
    }
}

#[test]
fn flag_no_scan_key_is_not_redundant() {
    assert!(!is_redundant_scan(None, None));
    assert!(!is_redundant_scan(None, Some(&scan_key(&["a"]))));
}

#[test]
fn flag_equal_scan_keys_are_redundant() {
    let key = scan_key(&["a"]);
    let mut last = scan_key(&["a"]);
    last.stream_ended = true;
    last.tool_calls_in_flight = true;
    assert!(is_redundant_scan(Some(&key), Some(&last)));
}

#[test]
fn flag_an_empty_scan_key_is_redundant() {
    let key = scan_key(&["", ""]);
    assert!(is_redundant_scan(Some(&key), None));
}

#[test]
fn flag_different_scan_keys_are_not_redundant() {
    let key = scan_key(&["a"]);
    let last = scan_key(&["b"]);
    assert!(!is_redundant_scan(Some(&key), Some(&last)));
    assert!(!is_redundant_scan(Some(&key), None));
}

#[test]
fn flag_a_key_with_tool_calls_has_something_to_scan() {
    let mut key = scan_key(&[""]);
    key.tool_calls = vec!["call_1".to_owned()];
    assert!(!is_redundant_scan(Some(&key), None));
}

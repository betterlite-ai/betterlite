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
    HttpErrorDetail, StreamErrorEmission, StreamHttpError, a2a_jsonrpc_error_chunk, a2a_request_id,
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
        let count = responses_so_far.map_or(0, <[GuardrailValue]>::len);
        let summary = format!("started={stream_started};items={count}");
        block_chunks(&self.block, &summary)
    }

    fn build_stream_error_items(
        &self,
        failure: &Self::Failure,
        responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<GuardrailValue>> {
        self.error_items
            .as_ref()
            .map(|items| echo_error_items(items, responses_so_far, failure))
    }
}

fn block_chunks(behavior: &BlockBehavior, summary: &str) -> Option<Vec<Vec<u8>>> {
    match behavior {
        BlockBehavior::Missing => None,
        BlockBehavior::Fixed(chunks) => Some(chunks.clone()),
        BlockBehavior::Summary => Some(vec![summary.as_bytes().to_vec()]),
    }
}

fn echo_error_items(
    items: &[GuardrailValue],
    carried: Option<&[GuardrailValue]>,
    failure: &GuardrailFailure,
) -> Vec<GuardrailValue> {
    let count = carried.map(<[GuardrailValue]>::len).unwrap_or_default();
    let status = match failure {
        GuardrailFailure::Http { status, .. } | GuardrailFailure::HttpObject { status, .. } => {
            *status
        }
        GuardrailFailure::ModifyResponse(_)
        | GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Other(_) => 0,
    };
    let mut echoed = items.to_vec();
    echoed.push(text(&format!("status={status};items={count};{failure}")));
    echoed
}

fn probe() -> ProbeTranslation {
    ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: None,
    }
}

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
    let resolved =
        resolve_endpoint_translation(&registry, &routes, Some("/completions"), Some(&chunk("hi")));
    assert!(resolved.is_none());
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

#[test]
fn emit_stream_error_display_prints_the_detail() {
    let text_error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("denied".to_owned()),
    };
    assert_eq!(text_error.to_string(), "denied");
    let object_error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Object(BTreeMap::from([("message".to_owned(), text("denied"))])),
    };
    assert_eq!(object_error.to_string(), r#"{"message":"denied"}"#);
}

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
    let so_far = [chunk("hi")];
    let chunks = handle_streaming_block(
        &translation,
        GuardrailFailure::Other("blocked".to_owned()),
        false,
        &so_far,
    )
    .unwrap();
    assert_eq!(chunks, vec![b"started=false;items=1".to_vec()]);
}

#[test]
fn emit_block_continues_the_message_when_the_stream_started() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Summary,
        error_items: None,
    };
    let so_far = [chunk("hi"), chunk("there")];
    let chunks = handle_streaming_block(
        &translation,
        GuardrailFailure::Other("blocked".to_owned()),
        true,
        &so_far,
    )
    .unwrap();
    assert_eq!(chunks, vec![b"started=true;items=2".to_vec()]);
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
    let so_far = [parsed(r#"{"id": 9}"#)];
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::SendMessage),
        responses_so_far: &so_far,
        request_data: &request_data,
        translation: Some(&translation),
        stream_started: false,
        responses_yielded: None,
    };
    let items = emit_streaming_http_error(error, &emission).unwrap();
    assert_eq!(items.len(), 1);
    let expected = parsed(
        r#"{"jsonrpc": "2.0", "id": 9, "error": {"code": -32603, "message": "blocked", "data": {}}}"#,
    );
    assert_eq!(items.first(), Some(&expected));
}

#[test]
fn emit_http_error_a2a_wins_over_a_started_stream_translation() {
    let translation = ProbeTranslation {
        block: BlockBehavior::Missing,
        error_items: Some(vec![text("translation")]),
    };
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let so_far = [parsed(r#"{"id": 9}"#)];
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::AsendMessage),
        responses_so_far: &so_far,
        request_data: &request_data,
        translation: Some(&translation),
        stream_started: true,
        responses_yielded: None,
    };
    let items = emit_streaming_http_error(error, &emission).unwrap();
    let expected = parsed(
        r#"{"jsonrpc": "2.0", "id": 9, "error": {"code": -32603, "message": "blocked", "data": {}}}"#,
    );
    assert_eq!(items, vec![expected]);
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
    let so_far = [chunk("a"), chunk("b"), chunk("c")];
    let yielded = [chunk("sent")];
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::Completion),
        responses_so_far: &so_far,
        request_data: &request_data,
        translation: Some(&translation),
        stream_started: true,
        responses_yielded: Some(&yielded),
    };
    let items = emit_streaming_http_error(error, &emission).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items.get(1), Some(&text("status=422;items=1;blocked")));
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
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::Completion),
        responses_so_far: &[],
        request_data: &request_data,
        translation: Some(&translation),
        stream_started: false,
        responses_yielded: None,
    };
    let result = emit_streaming_http_error(error.clone(), &emission);
    assert_eq!(result, Err(error));
}

#[test]
fn emit_http_error_without_a_translation_gives_back_the_error() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::Completion),
        responses_so_far: &[],
        request_data: &request_data,
        translation: None::<&ProbeTranslation>,
        stream_started: true,
        responses_yielded: None,
    };
    let result = emit_streaming_http_error(error.clone(), &emission);
    assert_eq!(result, Err(error));
}

#[test]
fn emit_http_error_without_error_items_gives_back_the_error() {
    let translation = probe();
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Text("blocked".to_owned()),
    };
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::Completion),
        responses_so_far: &[],
        request_data: &request_data,
        translation: Some(&translation),
        stream_started: true,
        responses_yielded: None,
    };
    let result = emit_streaming_http_error(error.clone(), &emission);
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
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::Completion),
        responses_so_far: &[],
        request_data: &request_data,
        translation: Some(&translation),
        stream_started: true,
        responses_yielded: None,
    };
    let items = emit_streaming_http_error(error, &emission).unwrap();
    assert_eq!(items, vec![text("status=403;items=0;denied")]);
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
    let request_data = BTreeMap::new();
    let emission = StreamErrorEmission {
        call_type: Some(CallType::Completion),
        responses_so_far: &[],
        request_data: &request_data,
        translation: Some(&translation),
        stream_started: true,
        responses_yielded: None,
    };
    let items = emit_streaming_http_error(error, &emission).unwrap();
    assert_eq!(
        items,
        vec![text(r#"status=403;items=0;{"message":"denied"}"#)]
    );
}

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
fn a2a_error_chunk_keeps_a_present_null_error_as_the_message() {
    let error = StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Object(BTreeMap::from([
            ("error".to_owned(), GuardrailValue::Null),
            ("message".to_owned(), text("ignored")),
        ])),
    };
    let chunk = a2a_jsonrpc_error_chunk(&error, GuardrailValue::Null);
    assert_eq!(
        chunk,
        parsed(
            r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": null, "data": {}}}"#
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

include!("tests/transform_chunk.rs");
include!("tests/stream_items.rs");
include!("tests/flags.rs");

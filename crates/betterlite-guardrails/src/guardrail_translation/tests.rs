#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    BaseTranslation, StreamTransformSink, StreamingScanKey, TranslationCall,
    merge_user_key_metadata, transform_user_key_metadata,
};

fn call() -> TranslationCall<'static, ()> {
    TranslationCall {
        user_api_key_dict: None,
        request_data: None,
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn scan_key(texts: &[&str], tool_calls: &[&str]) -> StreamingScanKey {
    StreamingScanKey {
        texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        tool_calls: tool_calls.iter().map(|call| (*call).to_owned()).collect(),
        stream_ended: false,
        tool_calls_in_flight: false,
    }
}

#[test]
fn stream_transform_sink_defaults_are_empty() {
    let mut sink = StreamTransformSink::default();
    assert!(sink.mutated_text_per_choice.is_empty());
    assert!(sink.holdback_per_choice.is_empty());
    sink.mutated_text_per_choice
        .insert(0, "rewritten".to_owned());
    sink.holdback_per_choice.insert(0, 3);
    assert_eq!(
        sink.mutated_text_per_choice.get(&0).map(String::as_str),
        Some("rewritten")
    );
    assert_eq!(sink.holdback_per_choice.get(&0), Some(&3));
}

#[rstest]
#[case::empty_texts_and_no_tool_calls(&[], &[], false, false, true)]
#[case::one_empty_text(&[""], &[], false, false, true)]
#[case::one_non_empty_text(&["hello"], &[], false, false, false)]
#[case::tool_calls_pending(&[], &["call-1"], false, true, false)]
#[case::ended_stream_with_empty_payload(&[], &[], true, false, false)]
#[case::ended_stream_with_text(&["hello"], &[], true, false, false)]
fn scan_key_has_nothing_to_scan(
    #[case] texts: &[&str],
    #[case] tool_calls: &[&str],
    #[case] stream_ended: bool,
    #[case] tool_calls_in_flight: bool,
    #[case] expected: bool,
) {
    let mut key = scan_key(texts, tool_calls);
    key.stream_ended = stream_ended;
    key.tool_calls_in_flight = tool_calls_in_flight;
    assert_eq!(key.has_nothing_to_scan(), expected);
}

#[test]
fn scan_key_equality_ignores_protocol_flags() {
    let mut first = scan_key(&["hello"], &[]);
    let mut second = scan_key(&["hello"], &[]);
    first.stream_ended = true;
    first.tool_calls_in_flight = true;
    second.stream_ended = false;
    second.tool_calls_in_flight = false;
    assert_eq!(first, second);
}

#[test]
fn scan_key_equality_compares_texts_and_tool_calls() {
    assert_ne!(scan_key(&["hello"], &[]), scan_key(&["other"], &[]));
    assert_ne!(scan_key(&[], &["call-1"]), scan_key(&[], &["call-2"]));
    assert_eq!(scan_key(&["a", "b"], &["c"]), scan_key(&["a", "b"], &["c"]));
}

#[test]
fn user_metadata_transform_prefixes_every_key() {
    let user_api_key_dict = BTreeMap::from([
        ("user_id".to_owned(), text("u-1")),
        ("team_id".to_owned(), text("t-1")),
        ("user_api_key_alias".to_owned(), text("alias")),
    ]);
    let transformed = transform_user_key_metadata(Some(&user_api_key_dict));
    assert_eq!(transformed.len(), 3);
    assert_eq!(transformed.get("user_api_key_user_id"), Some(&text("u-1")));
    assert_eq!(transformed.get("user_api_key_team_id"), Some(&text("t-1")));
    assert_eq!(transformed.get("user_api_key_alias"), Some(&text("alias")));
}

#[test]
fn user_metadata_transform_skips_null_and_internal_keys() {
    let user_api_key_dict = BTreeMap::from([
        ("user_id".to_owned(), text("u-1")),
        ("team_id".to_owned(), GuardrailValue::Null),
        ("_internal".to_owned(), text("hidden")),
    ]);
    let transformed = transform_user_key_metadata(Some(&user_api_key_dict));
    assert_eq!(
        transformed,
        BTreeMap::from([("user_api_key_user_id".to_owned(), text("u-1"))])
    );
}

#[test]
fn user_metadata_transform_without_key_dict_is_empty() {
    assert!(transform_user_key_metadata(None).is_empty());
}

#[test]
fn metadata_merge_creates_the_metadata_bucket() {
    let mut request_data = BTreeMap::from([("model".to_owned(), text("gpt-4o"))]);
    let user_api_key_dict = BTreeMap::from([("user_id".to_owned(), text("u-1"))]);
    merge_user_key_metadata(&mut request_data, Some(&user_api_key_dict));
    assert_eq!(
        request_data.get("metadata"),
        Some(&object(&[("user_api_key_user_id", text("u-1"))]))
    );
}

#[test]
fn metadata_merge_prefers_litellm_metadata_when_present() {
    let mut request_data = BTreeMap::from([
        ("model".to_owned(), text("gpt-4o")),
        ("litellm_metadata".to_owned(), object(&[])),
    ]);
    let user_api_key_dict = BTreeMap::from([("user_id".to_owned(), text("u-1"))]);
    merge_user_key_metadata(&mut request_data, Some(&user_api_key_dict));
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&object(&[("user_api_key_user_id", text("u-1"))]))
    );
    assert!(!request_data.contains_key("metadata"));
}

#[test]
fn metadata_merge_keeps_existing_keys() {
    let mut request_data = BTreeMap::from([(
        "metadata".to_owned(),
        object(&[("user_api_key_user_id", text("existing"))]),
    )]);
    let user_api_key_dict = BTreeMap::from([
        ("user_id".to_owned(), text("u-1")),
        ("team_id".to_owned(), text("t-1")),
    ]);
    merge_user_key_metadata(&mut request_data, Some(&user_api_key_dict));
    assert_eq!(
        request_data.get("metadata"),
        Some(&object(&[
            ("user_api_key_user_id", text("existing")),
            ("user_api_key_team_id", text("t-1")),
        ]))
    );
}

#[test]
fn metadata_merge_replaces_a_non_object_bucket() {
    let mut request_data = BTreeMap::from([("metadata".to_owned(), text("not-an-object"))]);
    let user_api_key_dict = BTreeMap::from([("user_id".to_owned(), text("u-1"))]);
    merge_user_key_metadata(&mut request_data, Some(&user_api_key_dict));
    assert_eq!(
        request_data.get("metadata"),
        Some(&object(&[("user_api_key_user_id", text("u-1"))]))
    );
}

#[test]
fn metadata_merge_skips_when_user_metadata_is_empty() {
    let mut request_data = BTreeMap::from([("model".to_owned(), text("gpt-4o"))]);
    let user_api_key_dict = BTreeMap::from([("team_id".to_owned(), GuardrailValue::Null)]);
    merge_user_key_metadata(&mut request_data, Some(&user_api_key_dict));
    assert_eq!(
        request_data,
        BTreeMap::from([("model".to_owned(), text("gpt-4o"))])
    );
}

#[derive(Debug, thiserror::Error)]
#[error("the marker translation never fails")]
struct MarkerFailure;

struct MarkerTranslation;

impl BaseTranslation for MarkerTranslation {
    type Guardrail = String;
    type LogTarget = ();
    type Failure = MarkerFailure;

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        data.insert("input_scanned".to_owned(), GuardrailValue::Boolean(true));
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        if let Some(request_data) = call.request_data {
            request_data.insert("response".to_owned(), response);
        }
        ready(Ok(text(guardrail)))
    }
}

#[tokio::test]
async fn translation_runs_required_hooks_and_keeps_stream_chunks() {
    let translation = MarkerTranslation;
    let guardrail = "[redacted]".to_owned();
    let mut data = BTreeMap::from([("model".to_owned(), text("gpt-4o"))]);
    translation
        .process_input_messages(&mut data, &guardrail, None)
        .await
        .unwrap();
    assert_eq!(
        data,
        BTreeMap::from([
            ("input_scanned".to_owned(), GuardrailValue::Boolean(true)),
            ("model".to_owned(), text("gpt-4o")),
        ])
    );

    let mut request_data = BTreeMap::new();
    let output_call = TranslationCall {
        user_api_key_dict: None,
        request_data: Some(&mut request_data),
        logging_obj: None,
        stream_transform_sink: None,
        deliver_ended_stream_rewrites: false,
    };
    let response = translation
        .process_output_response(text("answer"), &guardrail, output_call)
        .await
        .unwrap();
    assert_eq!(response, text("[redacted]"));
    assert_eq!(
        request_data,
        BTreeMap::from([("response".to_owned(), text("answer"))])
    );

    let mut responses_so_far = vec![text("chunk")];
    let original_chunks = responses_so_far.clone();
    translation
        .process_streaming_response(&mut responses_so_far, &guardrail, call())
        .await
        .unwrap();
    assert_eq!(responses_so_far, original_chunks);
}

#[test]
fn translation_defaults_have_no_provider_specific_extras() {
    let translation = MarkerTranslation;
    assert_eq!(translation.get_streaming_scan_key(&[]), None);
    assert_eq!(
        translation.build_block_sse_chunks(&MarkerFailure, false, None),
        None
    );
    assert_eq!(
        translation.build_stream_error_items(&MarkerFailure, None),
        None
    );
    assert_eq!(translation.get_structured_messages(&BTreeMap::new()), None);
    assert!(
        translation
            .extract_request_tool_names(&BTreeMap::new())
            .is_empty()
    );
    assert_eq!(
        translation.post_call_hook_response(text("answer")),
        text("answer")
    );
}

#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::slice::from_ref;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use uuid::Uuid;

use crate::vertex_batch_rows::correlation::{CorrelationInput, decode_batch_correlation};
use crate::vertex_batch_rows::decode::{
    decode_vertex_batch_row, is_embeddings_output_row, is_generate_output_row,
    split_vertex_batch_key,
};
use crate::vertex_batch_rows::embeddings::{
    VERTEX_AI_ERROR, openai_batch_output_row, regroup_embeddings,
};
use crate::vertex_batch_rows::error::BatchRowsError;
use crate::vertex_batch_rows::labels::{
    GCP_LABEL_VALUE_MAX_LEN, RAW_LABEL, custom_id_from_labels, decode_gcp_label_value_chunks,
    set_custom_id_labels,
};
use crate::vertex_batch_rows::paths::{is_passthrough_managed_gcs_url, model_from_managed_gcs_url};
use crate::vertex_batch_rows::types::{
    OpenAiBatchOutput, OpenAiBatchOutputError, RowField, VertexBatchRow,
};

fn row(source: &str) -> VertexBatchRow {
    decode_vertex_batch_row(source.as_bytes()).unwrap()
}

fn labels(pairs: &[(&str, &str)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

fn written_labels(custom_id: &str) -> IndexMap<String, Json> {
    let mut written = IndexMap::new();
    set_custom_id_labels(&mut written, custom_id);
    written
}

fn label_text(written: &IndexMap<String, Json>, name: &str) -> String {
    let Json::Text(text) = written.get(name).unwrap() else {
        panic!("the {name} label is not text");
    };
    text.clone()
}

fn ids() -> impl FnMut() -> Uuid {
    let mut next = 0_u128;
    move || {
        next = next.saturating_add(1);
        Uuid::from_u128(next)
    }
}

fn regroup(rows: &[VertexBatchRow], model: Option<&str>) -> Vec<OpenAiBatchOutput> {
    regroup_embeddings(rows, model, &mut ids()).unwrap()
}

fn output_value(output: &OpenAiBatchOutput) -> Json {
    serde_json::from_value(serde_json::to_value(output).unwrap()).unwrap()
}

#[rstest]
#[case::lowers_and_replaces("H\u{e9}llo_WORLD.x", "h_llo_world_x")]
#[case::keeps_allowed_punctuation("a-b_c9", "a-b_c9")]
#[case::drops_unicode_symbols("w\u{f6}rld\u{2713}", "w_rld_")]
#[case::folds_turkish_capital_i("\u{130}stanbul", "i_stanbul")]
#[case::keeps_empty("", "")]
fn set_custom_id_labels_writes_the_gcp_safe_identifier(
    #[case] custom_id: &str,
    #[case] expected: &str,
) {
    let written = written_labels(custom_id);
    assert_eq!(label_text(&written, "litellm_custom_id"), expected);
}

#[test]
fn set_custom_id_labels_truncates_the_identifier_at_the_gcp_limit() {
    let long = "a".repeat(GCP_LABEL_VALUE_MAX_LEN.saturating_add(5));
    assert_eq!(
        label_text(&written_labels(&long), "litellm_custom_id").len(),
        GCP_LABEL_VALUE_MAX_LEN
    );
}

#[rstest]
#[case::short("req-1", vec!["b32_ojsxcljr"])]
#[case::empty("", vec!["b32_"])]
#[case::unicode("H\u{e9}llo_WORLD.x", vec!["b32_jdb2s3dmn5pvot2sjrcc46a"])]
fn set_custom_id_labels_writes_the_raw_chunks(
    #[case] custom_id: &str,
    #[case] expected: Vec<&str>,
) {
    let written = written_labels(custom_id);
    let mut chunks = vec![label_text(&written, RAW_LABEL)];
    for index in 1..=written.len().saturating_sub(2) {
        let name = format!("litellm_custom_id_raw_{index}");
        chunks.push(label_text(&written, &name));
    }
    assert_eq!(chunks, expected);
}

#[test]
fn set_custom_id_labels_numbers_every_chunk_after_the_first() {
    let custom_id = "x".repeat(74);
    let written = written_labels(&custom_id);
    let payloads: Vec<String> = [
        RAW_LABEL,
        "litellm_custom_id_raw_1",
        "litellm_custom_id_raw_2",
    ]
    .iter()
    .map(|name| label_text(&written, name))
    .collect();
    assert_eq!(
        payloads,
        vec![
            "b32_pb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4",
            "b32_hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6",
            "b32_a",
        ]
    );
    assert_eq!(custom_id_from_labels(&written).unwrap(), custom_id);
}

#[test]
fn set_custom_id_labels_splits_payloads_after_the_prefix() {
    let written = written_labels(&"x".repeat(40));
    let first = label_text(&written, RAW_LABEL);
    let second = label_text(&written, "litellm_custom_id_raw_1");
    assert_eq!(
        vec![first, second],
        vec![
            "b32_pb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4hq6dypb4",
            "b32_hq6dy",
        ]
    );
    assert!(written.values().all(|value| match value {
        Json::Text(text) => text.len() <= GCP_LABEL_VALUE_MAX_LEN,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => false,
    }));
}

#[rstest]
#[case::round_trip(vec!["b32_jbswy3dp"], Some("Hello"))]
#[case::payload_only(vec!["b32_"], Some(""))]
#[case::no_chunks(vec![], Some(""))]
#[case::missing_prefix(vec!["raw"], None)]
#[case::invalid_utf8_bytes(vec!["b32_zz"], None)]
#[case::rejects_bad_padding(vec!["b32_a"], None)]
#[case::ignores_trailing_bits(vec!["b32_ab"], Some("\u{0}"))]
#[case::joins_whole_quanta(vec!["b32_jbswy3dp", "b32_orugs"], Some("Hellothi"))]
#[case::accepts_python_padding(vec!["b32_jbswy3dp", "b32_orugs==="], Some("Hellothi"))]
#[case::rejects_unaligned_join(vec!["b32_jbswy3dp", "b32_orugs", "b32_mzxw6"], None)]
#[case::rejects_uppercase_prefix(vec!["B32_JB"], None)]
fn decode_gcp_label_value_chunks_matches_the_litellm_decoder(
    #[case] chunks: Vec<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(decode_gcp_label_value_chunks(&chunks).as_deref(), expected);
}

#[test]
fn set_custom_id_labels_round_trips_a_unicode_identifier() {
    let written = written_labels("H\u{e9}llo World.req-1");
    assert_eq!(written.len(), 2);
    assert_eq!(
        custom_id_from_labels(&written).unwrap(),
        "H\u{e9}llo World.req-1"
    );
}

#[test]
fn set_custom_id_labels_round_trips_an_identifier_across_chunks() {
    let written = written_labels(&"x".repeat(40));
    assert!(written.contains_key("litellm_custom_id_raw_1"));
    assert_eq!(custom_id_from_labels(&written).unwrap(), "x".repeat(40));
}

#[test]
fn custom_id_from_labels_does_not_percent_decode_the_raw_label() {
    let written = written_labels("a%23b");
    assert_eq!(custom_id_from_labels(&written).unwrap(), "a%23b");
    let encoded = labels(&[(RAW_LABEL, "b32_mestem3c")]);
    assert_eq!(custom_id_from_labels(&encoded).unwrap(), "a%23b");
}

#[test]
fn custom_id_from_labels_reads_the_decoded_raw_label() {
    let written = labels(&[(RAW_LABEL, "b32_jbswy3dp")]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "Hello");
}

#[test]
fn custom_id_from_labels_falls_back_to_the_first_raw_label() {
    let written = labels(&[(RAW_LABEL, "b32_zz"), ("litellm_custom_id", "lossy")]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "b32_zz");
}

#[test]
fn custom_id_from_labels_ignores_an_empty_raw_label() {
    let written = labels(&[(RAW_LABEL, ""), ("litellm_custom_id", "ordinary")]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "ordinary");
}

#[test]
fn custom_id_from_labels_reads_the_ordinary_label() {
    let written = labels(&[("litellm_custom_id", "ordinary")]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "ordinary");
}

#[test]
fn custom_id_from_labels_reports_unknown_without_any_label() {
    assert_eq!(custom_id_from_labels(&IndexMap::new()).unwrap(), "unknown");
}

#[test]
fn custom_id_from_labels_orders_numbered_chunks_numerically() {
    let written = labels(&[
        (RAW_LABEL, "b32_"),
        ("litellm_custom_id_raw_10", "b32_mzxw6ztf"),
        ("litellm_custom_id_raw_2", "b32_jbswy3dp"),
    ]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "Hellofoofe");
}

#[test]
fn custom_id_from_labels_falls_back_when_the_joined_chunks_do_not_decode() {
    let written = labels(&[
        (RAW_LABEL, "b32_jbswy3dp"),
        ("litellm_custom_id_raw_10", "b32_mzxw6"),
        ("litellm_custom_id_raw_2", "b32_orugs"),
    ]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "b32_jbswy3dp");
}

#[test]
fn custom_id_from_labels_keeps_numbered_chunks_across_a_gap() {
    let written = labels(&[
        (RAW_LABEL, "b32_jbswy3dp"),
        ("litellm_custom_id_raw_10", "b32_mzxw6"),
        ("litellm_custom_id_raw_2", "b32_orugs"),
        ("litellm_custom_id_raw_7", "b32_a"),
    ]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "b32_jbswy3dp");
}

#[test]
fn custom_id_from_labels_keeps_a_zero_numbered_chunk_after_the_raw_one() {
    let written = labels(&[
        (RAW_LABEL, "b32_"),
        ("litellm_custom_id_raw_0", "b32_mzxw6ztf"),
    ]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "foofe");
}

#[rstest]
#[case::letter("litellm_custom_id_raw_x")]
#[case::empty("litellm_custom_id_raw_")]
#[case::plus_sign("litellm_custom_id_raw_+1")]
fn custom_id_from_labels_ignores_a_suffix_that_is_not_numeric(#[case] name: &str) {
    let written = labels(&[(RAW_LABEL, "b32_jbswy3dp"), (name, "b32_a")]);
    assert_eq!(custom_id_from_labels(&written).unwrap(), "Hello");
}

#[test]
fn custom_id_from_labels_rejects_a_non_text_raw_label() {
    let mut written = IndexMap::new();
    written.insert(
        String::from(RAW_LABEL),
        Json::Number(serde_json::Number::from(5_i32)),
    );
    let error = custom_id_from_labels(&written).unwrap_err();
    assert!(matches!(error, BatchRowsError::Label(label) if label == RAW_LABEL));
}

#[test]
fn custom_id_from_labels_rejects_a_non_text_ordinary_label() {
    let mut written = IndexMap::new();
    written.insert(String::from("litellm_custom_id"), Json::Null);
    let error = custom_id_from_labels(&written).unwrap_err();
    assert!(matches!(error, BatchRowsError::Label(label) if label == "litellm_custom_id"));
}

#[test]
fn custom_id_from_labels_rejects_a_non_text_numbered_chunk() {
    let mut written = labels(&[(RAW_LABEL, "b32_jbswy3dp")]);
    written.insert(String::from("litellm_custom_id_raw_1"), Json::Bool(true));
    let error = custom_id_from_labels(&written).unwrap_err();
    assert!(matches!(error, BatchRowsError::Label(label) if label == "litellm_custom_id_raw_1"));
}

#[test]
fn custom_id_from_labels_rejects_a_chunk_number_past_u64() {
    let chunk = "litellm_custom_id_raw_99999999999999999999999";
    let written = labels(&[(RAW_LABEL, "b32_jbswy3dp"), (chunk, "b32_orugs")]);
    let error = custom_id_from_labels(&written).unwrap_err();
    assert!(matches!(error, BatchRowsError::Label(label) if label == chunk));
}

#[test]
fn a_row_decodes_its_labels_and_optional_fields() {
    let decoded = row(
        r#"{"key": "a", "request": {"labels": {"litellm_custom_id": "req-1"}}, "status": "ok", "processed_time": "now"}"#,
    );
    let request = decoded.request.object().unwrap();
    let Some(Json::Object(labels)) = request.get("labels") else {
        panic!("the request carries no labels");
    };
    assert_eq!(
        labels.get("litellm_custom_id"),
        Some(&Json::Text("req-1".to_owned()))
    );
    assert_eq!(decoded.key.as_deref(), Some("a"));
    assert_eq!(decoded.status.as_deref(), Some("ok"));
    assert_ne!(decoded.processed_time, RowField::Absent);
}

#[rstest]
#[case::null_processed_time(r#"{"request": {}, "response": {}, "processed_time": null}"#, true)]
#[case::absent_processed_time(r#"{"request": {}, "response": {}}"#, false)]
fn a_null_processed_time_still_counts_as_present(#[case] source: &str, #[case] present: bool) {
    assert_eq!(row(source).processed_time != RowField::Absent, present);
}

#[rstest]
#[case::not_json("{")]
#[case::not_an_object("[1]")]
#[case::empty_array("[]")]
#[case::array_of_nulls("[null, null]")]
#[case::padded_array("\n [] ")]
#[case::null_row("null")]
#[case::text_row("\"row\"")]
fn decode_vertex_batch_row_rejects_bytes_that_are_not_a_row(#[case] source: &str) {
    decode_vertex_batch_row(source.as_bytes()).unwrap_err();
}

#[test]
fn decode_vertex_batch_row_rejects_invalid_utf8() {
    decode_vertex_batch_row(&[0xFF, 0xFE]).unwrap_err();
}

#[rstest]
#[case::by_embedding(r#"{"request": {}, "response": {"embedding": {"values": [1]}}}"#, true)]
#[case::by_failed_content(r#"{"request": {"content": {"parts": []}}, "status": "failed"}"#, true)]
#[case::without_request(r#"{"response": {"embedding": {"values": [1]}}}"#, false)]
#[case::without_content(r#"{"request": {}, "status": "failed"}"#, false)]
#[case::without_status(r#"{"request": {"content": {}}}"#, false)]
#[case::null_request(
    r#"{"request": null, "response": {"embedding": {"values": [1]}}}"#,
    true
)]
#[case::embedding_not_an_object(
    r#"{"request": {}, "status": "failed", "response": {"embedding": "x"}}"#,
    false
)]
fn is_embeddings_output_row_matches_the_row_shape(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(is_embeddings_output_row(&row(source)), expected);
}

#[rstest]
#[case::candidates(
    r#"{"request": {}, "response": {"candidates": []}, "processed_time": ""}"#,
    true
)]
#[case::prompt_feedback(
    r#"{"request": {}, "response": {"promptFeedback": {}}, "processed_time": ""}"#,
    true
)]
#[case::status(
    r#"{"request": {}, "response": {}, "processed_time": "", "status": "bad"}"#,
    true
)]
#[case::empty_status(r#"{"request": {}, "response": {}, "processed_time": ""}"#, false)]
#[case::without_processed_time(r#"{"request": {}, "response": {}}"#, false)]
#[case::without_request(r#"{"response": {}, "processed_time": ""}"#, false)]
#[case::without_response(r#"{"request": {}, "processed_time": "", "status": "bad"}"#, false)]
#[case::null_processed_time(
    r#"{"request": {}, "response": {}, "processed_time": null, "status": "bad"}"#,
    true
)]
fn is_generate_output_row_matches_the_row_shape(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(is_generate_output_row(&row(source)), expected);
}

#[rstest]
#[case::key("a%23b#1/2", "a#b", 1, 2)]
#[case::plain_key("plain", "plain", 0, 1)]
#[case::empty_key("", "", 0, 1)]
#[case::second_hash("a#1/2#3", "a#1/2#3", 0, 1)]
#[case::not_a_number("a#x/2", "a#x/2", 0, 1)]
#[case::signed_index("a#+1/2", "a#+1/2", 0, 1)]
#[case::signed_total("a#1/+2", "a#1/+2", 0, 1)]
#[case::decoded_index("%C3%A9#0/1", "\u{e9}", 0, 1)]
#[case::empty_identifier("#1/2", "", 1, 2)]
#[case::leading_zeros("a#01/02", "a", 1, 2)]
fn split_vertex_batch_key_reads_the_fan_out_suffix(
    #[case] key: &str,
    #[case] custom_id: &str,
    #[case] index: u32,
    #[case] total: u32,
) {
    let source = format!(r#"{{"key": "{key}"}}"#);
    let split = split_vertex_batch_key(&row(&source)).unwrap();
    assert_eq!(split.custom_id, custom_id);
    assert_eq!(split.index, index);
    assert_eq!(split.total, total);
}

#[test]
fn split_vertex_batch_key_falls_back_to_the_labels_without_a_key() {
    let source = r#"{"request": {"labels": {"litellm_custom_id_raw": "b32_jbswy3dp"}}}"#;
    let split = split_vertex_batch_key(&row(source)).unwrap();
    assert_eq!(split.custom_id, "Hello");
    assert_eq!(split.index, 0);
    assert_eq!(split.total, 1);
}

#[test]
fn split_vertex_batch_key_reports_unknown_without_a_key_or_labels() {
    let split = split_vertex_batch_key(&row(r#"{"request": {}}"#)).unwrap();
    assert_eq!(split.custom_id, "unknown");
}

#[test]
fn a_key_wins_over_the_labels() {
    let source =
        r#"{"key": "from-key", "request": {"labels": {"litellm_custom_id": "from-label"}}}"#;
    assert_eq!(
        split_vertex_batch_key(&row(source)).unwrap().custom_id,
        "from-key"
    );
}

#[test]
fn a_null_key_falls_back_to_the_labels() {
    let source = r#"{"key": null, "request": {"labels": {"litellm_custom_id": "from-label"}}}"#;
    assert_eq!(
        split_vertex_batch_key(&row(source)).unwrap().custom_id,
        "from-label"
    );
}

#[test]
fn a_key_is_percent_decoded() {
    assert_eq!(
        decode_batch_correlation(CorrelationInput::Key(b"a%23b%2Fc")).unwrap(),
        "a#b/c"
    );
}

#[test]
fn a_key_that_is_not_utf8_is_refused() {
    decode_batch_correlation(CorrelationInput::Key(&[0xFF])).unwrap_err();
}

#[test]
fn a_percent_decoded_key_that_is_not_utf8_is_refused() {
    decode_batch_correlation(CorrelationInput::Key(b"%FF")).unwrap_err();
}

#[test]
fn label_chunks_decode_through_the_correlation_decoder() {
    let chunks: [&[u8]; 2] = [b"b32_jbswy3dp", b"b32_orugs"];
    assert_eq!(
        decode_batch_correlation(CorrelationInput::LabelChunks(&chunks)).unwrap(),
        "Hellothi"
    );
}

#[test]
fn label_chunks_fall_back_to_the_first_raw_label() {
    let chunks: [&[u8]; 1] = [b"b32_zz"];
    assert_eq!(
        decode_batch_correlation(CorrelationInput::LabelChunks(&chunks)).unwrap(),
        "b32_zz"
    );
}

#[test]
fn empty_label_chunks_decode_to_an_empty_identifier() {
    assert_eq!(
        decode_batch_correlation(CorrelationInput::LabelChunks(&[])).unwrap(),
        ""
    );
}

#[test]
fn label_chunks_that_are_not_utf8_are_refused() {
    let chunks: [&[u8]; 1] = [&[0xFF]];
    decode_batch_correlation(CorrelationInput::LabelChunks(&chunks)).unwrap_err();
}

#[test]
fn a_failed_label_chunk_that_is_not_utf8_is_refused() {
    let chunks: [&[u8]; 2] = [b"b32_jbswy3dp", &[0xFF]];
    decode_batch_correlation(CorrelationInput::LabelChunks(&chunks)).unwrap_err();
}

fn embedding_row(key: &str, values: &str, usage: &str) -> String {
    format!(
        r#"{{"key": "{key}", "request": {{}}, "status": "", "response": {{"embedding": {{"values": {values}}}{usage}}}}}"#
    )
}

fn embedding_rows(specs: &[(&str, &str, &str)]) -> Vec<VertexBatchRow> {
    specs
        .iter()
        .map(|(key, values, usage)| row(&embedding_row(key, values, usage)))
        .collect()
}

#[test]
fn shuffled_embedding_groups_keep_the_first_seen_identifier_order() {
    let rows = embedding_rows(&[
        ("id#1/3", "[2]", r#", "tokenCount": 4"#),
        ("other", "[9]", r#", "tokenCount": 1"#),
        (
            "id#0/3",
            "[1]",
            r#", "usageMetadata": {"promptTokenCount": 3}"#,
        ),
        ("id#2/3", "[3]", r#", "tokenCount": 5"#),
    ]);
    let outputs = regroup(&rows, None);
    assert_eq!(outputs.len(), 2);
    assert_eq!(outputs[0].custom_id, "id");
    assert_eq!(outputs[1].custom_id, "other");
    let body = &outputs[0].response.as_ref().unwrap().body;
    assert_eq!(
        body,
        &serde_json::from_str::<IndexMap<String, Json>>(
            r#"{"object":"list","model":"","data":[{"object":"embedding","embedding":[1],"index":0},{"object":"embedding","embedding":[2],"index":1},{"object":"embedding","embedding":[3],"index":2}],"usage":{"prompt_tokens":12,"completion_tokens":0,"completion_tokens_details":null,"prompt_tokens_details":null,"total_tokens":12}}"#
        )
        .unwrap()
    );
}

#[test]
fn embedding_groups_name_their_batch_model() {
    let rows = embedding_rows(&[("id#0/1", "[1]", "")]);
    let outputs = regroup(&rows, Some("text-embedding-005"));
    let body = &outputs[0].response.as_ref().unwrap().body;
    assert_eq!(
        body.get("model"),
        Some(&Json::Text("text-embedding-005".to_owned()))
    );
}

#[test]
fn embedding_values_may_be_a_base64_string() {
    let rows = embedding_rows(&[("id#0/1", r#""AAAA""#, "")]);
    let outputs = regroup(&rows, None);
    let body = &outputs[0].response.as_ref().unwrap().body;
    let Some(Json::List(data)) = body.get("data") else {
        panic!("the embedding data is not a list");
    };
    assert_eq!(
        data.first(),
        Some(
            &serde_json::from_str::<Json>(r#"{"object":"embedding","embedding":"AAAA","index":0}"#)
                .unwrap()
        )
    );
}

#[test]
fn a_missing_embedding_position_fails_the_whole_entry() {
    let rows = embedding_rows(&[("id#0/3", "[1]", ""), ("id#2/3", "[3]", "")]);
    let outputs = regroup(&rows, None);
    let error = outputs[0].error.as_ref().unwrap();
    assert!(outputs[0].response.is_none());
    assert_eq!(error.code, VERTEX_AI_ERROR);
    assert_eq!(
        error.message,
        "Vertex returned embeddings for input positions [0, 2] of the 3 requested"
    );
}

#[test]
fn a_duplicate_embedding_position_fails_the_whole_entry() {
    let rows = embedding_rows(&[("id#0/2", "[1]", ""), ("id#0/2", "[2]", "")]);
    let outputs = regroup(&rows, None);
    let error = outputs[0].error.as_ref().unwrap();
    assert_eq!(
        error.message,
        "Vertex returned embeddings for input positions [0, 0] of the 2 requested"
    );
}

#[test]
fn a_failed_element_reports_its_status() {
    let failed = row(
        r#"{"key": "id#1/2", "request": {}, "status": "service unavailable", "response": {"embedding": {"values": [1]}}}"#,
    );
    let ok = row(
        r#"{"key": "id#0/2", "request": {}, "status": "", "response": {"embedding": {"values": [2]}}}"#,
    );
    let outputs = regroup(&[failed, ok], None);
    let error = outputs[0].error.as_ref().unwrap();
    assert_eq!(error.code, VERTEX_AI_ERROR);
    assert_eq!(error.message, "service unavailable");
    assert!(outputs[0].response.is_none());
}

#[test]
fn a_failed_element_without_a_response_still_reports_its_status() {
    let source = r#"{"key": "id#0/1", "request": {}, "status": "quota"}"#;
    let outputs = regroup(&[row(source)], None);
    assert_eq!(outputs[0].error.as_ref().unwrap().message, "quota");
}

#[test]
fn an_embedding_row_without_a_response_is_refused() {
    let source = r#"{"key": "id#0/1", "request": {}, "status": ""}"#;
    let error = regroup_embeddings(&[row(source)], None, &mut ids()).unwrap_err();
    assert!(
        matches!(error, BatchRowsError::Shape(custom_id, field) if custom_id == "id" && field == "response")
    );
}

#[test]
fn embedding_values_that_are_not_numbers_are_refused() {
    let rows = embedding_rows(&[("id#0/1", r#"["a"]"#, "")]);
    let error = regroup_embeddings(&rows, None, &mut ids()).unwrap_err();
    assert!(
        matches!(error, BatchRowsError::Shape(custom_id, field) if custom_id == "id" && field == "embedding.values")
    );
}

#[test]
fn embedding_values_that_are_missing_are_refused() {
    let source = r#"{"key": "id#0/1", "request": {}, "status": "", "response": {"embedding": {}}}"#;
    let error = regroup_embeddings(&[row(source)], None, &mut ids()).unwrap_err();
    assert!(
        matches!(error, BatchRowsError::Shape(custom_id, field) if custom_id == "id" && field == "embedding.values")
    );
}

#[rstest]
#[case::past_u32(r#", "tokenCount": 4294967296"#)]
#[case::negative(r#", "tokenCount": -5"#)]
fn a_prompt_token_count_outside_u32_is_refused(#[case] usage: &str) {
    let rows = embedding_rows(&[("id#0/1", "[1]", usage)]);
    let error = regroup_embeddings(&rows, None, &mut ids()).unwrap_err();
    assert!(
        matches!(error, BatchRowsError::Shape(custom_id, field) if custom_id == "id" && field == "usage")
    );
}

#[test]
fn batch_output_rows_inject_the_given_id() {
    let body = serde_json::from_str::<IndexMap<String, Json>>(r#"{"object": "list"}"#).unwrap();
    let output =
        openai_batch_output_row(String::from("req-1"), Some(body), None, Uuid::from_u128(7));
    assert_eq!(output.id, "batch_req_00000000-0000-0000-0000-000000000007");
    assert_eq!(output.response.unwrap().status_code, 200);
}

#[test]
fn batch_output_rows_read_the_body_id_as_the_request_id() {
    let body = serde_json::from_str::<IndexMap<String, Json>>(r#"{"id": "chatcmpl-1"}"#).unwrap();
    let output =
        openai_batch_output_row(String::from("req-1"), Some(body), None, Uuid::from_u128(1));
    assert_eq!(
        output.response.unwrap().request_id,
        Json::Text(String::from("chatcmpl-1"))
    );
}

#[test]
fn batch_output_rows_copy_a_body_id_of_any_type() {
    let body = serde_json::from_str::<IndexMap<String, Json>>(r#"{"id": 5}"#).unwrap();
    let output =
        openai_batch_output_row(String::from("req-1"), Some(body), None, Uuid::from_u128(1));
    assert_eq!(
        output.response.unwrap().request_id,
        Json::Number(serde_json::Number::from(5_i32))
    );
}

#[test]
fn batch_output_rows_without_a_body_id_report_an_empty_request_id() {
    let body = serde_json::from_str::<IndexMap<String, Json>>(r#"{"object": "list"}"#).unwrap();
    let output =
        openai_batch_output_row(String::from("req-1"), Some(body), None, Uuid::from_u128(1));
    assert_eq!(
        output.response.unwrap().request_id,
        Json::Text(String::new())
    );
}

#[test]
fn batch_output_rows_report_an_error_without_a_response() {
    let error = OpenAiBatchOutputError {
        code: String::from("vertex_ai_error"),
        message: String::from("bad"),
    };
    let output =
        openai_batch_output_row(String::from("req-1"), None, Some(error), Uuid::from_u128(1));
    assert!(output.response.is_none());
    let value = output_value(&output);
    assert_eq!(
        value,
        serde_json::from_str::<Json>(
            r#"{"id":"batch_req_00000000-0000-0000-0000-000000000001","custom_id":"req-1","response":null,"error":{"code":"vertex_ai_error","message":"bad"}}"#
        )
        .unwrap()
    );
}

#[rstest]
#[case::managed("gs://b/litellm-vertex-files/passthrough/x", true)]
#[case::nested("gs://b/x/litellm-vertex-files/passthrough/y", true)]
#[case::first_prefix_wins(
    "gs://b/x/litellm-vertex-files/a/litellm-vertex-files/passthrough/y",
    false
)]
#[case::not_passthrough("gs://b/litellm-vertex-files/x", false)]
#[case::encoded_prefix("gs://b/%6Citemm-vertex-files/passthrough/x", false)]
#[case::plain("gs://b/x", false)]
fn is_passthrough_managed_gcs_url_reads_the_first_prefix(
    #[case] url: &str,
    #[case] expected: bool,
) {
    assert_eq!(is_passthrough_managed_gcs_url(url), expected);
}

#[rstest]
#[case::publisher(
    "https://x/publishers/google/models/gemini-1.5/?a=1",
    Some("gemini-1.5")
)]
#[case::other_publisher("https://x/publishers/anthropic/models/claude/rest", Some("claude"))]
#[case::encoded_publisher("https://x/p%75blishers/google/models/my%20model", Some("my model"))]
#[case::first_model_wins(
    "gs://b/litellm-vertex-files/x/publishers/google/models/first/publishers/google/models/second",
    Some("first")
)]
#[case::no_model_path("https://x/models/plain", None)]
fn model_from_managed_gcs_url_reads_the_first_model_segment(
    #[case] url: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(model_from_managed_gcs_url(url).as_deref(), expected);
}

#[rstest]
fn replays_the_vertex_batch_rows_corpus(#[files("fuzz/corpus/vertex_batch_rows/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(row) = decode_vertex_batch_row(&bytes) {
        if is_embeddings_output_row(&row) {
            drop(regroup_embeddings(from_ref(&row), None, &mut ids()));
        }
        drop(split_vertex_batch_key(&row));
    }
    let chunks: [&[u8]; 1] = [&bytes];
    drop(decode_batch_correlation(CorrelationInput::LabelChunks(
        &chunks,
    )));
    drop(decode_batch_correlation(CorrelationInput::Key(&bytes)));
}

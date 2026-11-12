#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::Value;

use super::{
    BatchIdRequest, BatchRecordsError, BatchRow, CreateBatchRequest, DecodedRows, ListBatchRequest,
    LiteLLMBatchCreateRequest, WireDecodeLimits, decode_batch_records,
};
use crate::batches::records::{BatchEndpoint, CompletionWindow, ExpiryAnchor, OutputExpiry};

fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0001_0000,
        max_event_bytes: 4096,
        max_items: 64,
    }
}

fn val(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn decode(content: &str) -> DecodedRows {
    decode_batch_records(content.as_bytes(), &limits()).unwrap()
}

fn row_values(decoded: &DecodedRows) -> Vec<Value> {
    decoded
        .rows
        .iter()
        .map(|row| serde_json::to_value(row).unwrap())
        .collect()
}

fn values(texts: &[&str]) -> Vec<Value> {
    texts.iter().map(|text| val(text)).collect()
}

#[rstest]
#[case::one_row_per_line("{\"id\":1}\n{\"id\":2}\n", &["{\"id\":1}", "{\"id\":2}"])]
#[case::no_trailing_newline("{\"id\":1}\n{\"id\":2}", &["{\"id\":1}", "{\"id\":2}"])]
#[case::object_spans_lines(
    "{\"id\":1,\n \"msg\":\"a\"}\n{\"id\":2}\n",
    &["{\"id\":1,\"msg\":\"a\"}", "{\"id\":2}"]
)]
#[case::blank_lines_between("\n\n{\"id\":1}\n\n\n{\"id\":2}\n\n", &["{\"id\":1}", "{\"id\":2}"])]
#[case::crlf("{\"id\":1}\r\n{\"id\":2}\r\n", &["{\"id\":1}", "{\"id\":2}"])]
#[case::trailing_spaces("{\"id\":1}   \n", &["{\"id\":1}"])]
#[case::python_separators("\u{1c}{\"id\":1}\u{1f}\n", &["{\"id\":1}"])]
#[case::scalars_and_arrays("1\n[2]\n\"three\"\nnull\n", &["1", "[2]", "\"three\"", "null"])]
#[case::empty("", &[])]
#[case::only_blank_lines("\n  \n\t\n", &[])]
fn splits_rows(#[case] content: &str, #[case] expected: &[&str]) {
    let decoded = decode(content);
    assert_eq!(row_values(&decoded), values(expected));
    assert!(decoded.unparsed.is_none());
}

#[rstest]
#[case::truncated_object("{\"id\":1}\n{\"id\":2,\n", &["{\"id\":1}"], "{\"id\":2,\n")]
#[case::extra_data_on_first_row("{\"id\":1} x\n{\"id\":2}\n", &[], "{\"id\":1} x\n{\"id\":2}\n")]
#[case::extra_data_keeps_earlier_rows(
    "{\"id\":1}\n{\"id\":2} x\n{\"id\":3}\n",
    &["{\"id\":1}"],
    "{\"id\":2} x\n{\"id\":3}\n"
)]
#[case::two_values_on_one_line("{\"id\":1} {\"id\":2}\n", &[], "{\"id\":1} {\"id\":2}\n")]
#[case::blank_lines_stay_in_the_buffer(
    "{\"id\":1}\n\n  {\"id\":2",
    &["{\"id\":1}"],
    "\n  {\"id\":2"
)]
#[case::bare_word("not json", &[], "not json")]
fn keeps_the_unparsed_tail(
    #[case] content: &str,
    #[case] expected: &[&str],
    #[case] preview: &str,
) {
    let decoded = decode(content);
    assert_eq!(row_values(&decoded), values(expected));
    let tail = decoded.unparsed.unwrap();
    assert_eq!(tail.preview, preview);
    assert!(!tail.error.to_string().is_empty());
}

#[test]
fn cuts_the_preview_at_one_hundred_characters() {
    let content = format!("[{}", "\u{e9}".repeat(150));
    let tail = decode(&content).unparsed.unwrap();
    assert_eq!(tail.preview.chars().count(), 100);
    assert!(content.starts_with(&tail.preview));
}

#[test]
fn scans_a_long_unfinished_value_in_one_pass() {
    let content = format!("[{}", "1,\n".repeat(20_000));
    let decoded = decode_batch_records(
        content.as_bytes(),
        &WireDecodeLimits {
            max_json_bytes: content.len(),
            max_event_bytes: content.len(),
            max_items: 4,
        },
    )
    .unwrap();
    assert!(decoded.rows.is_empty());
    assert!(decoded.unparsed.is_some());
}

#[test]
fn json_limit_is_inclusive() {
    let content = "{\"id\":1}\n";
    let at_limit = WireDecodeLimits {
        max_json_bytes: content.len(),
        ..limits()
    };
    assert_eq!(
        decode_batch_records(content.as_bytes(), &at_limit)
            .unwrap()
            .rows
            .len(),
        1
    );
    let below = WireDecodeLimits {
        max_json_bytes: content.len() - 1,
        ..limits()
    };
    assert!(matches!(
        decode_batch_records(content.as_bytes(), &below),
        Err(BatchRecordsError::JsonTooLarge { limit }) if limit == content.len() - 1
    ));
}

#[test]
fn row_limit_is_inclusive() {
    let row = "{\"id\":1}";
    let at_limit = WireDecodeLimits {
        max_event_bytes: row.len(),
        ..limits()
    };
    assert_eq!(
        decode_batch_records(row.as_bytes(), &at_limit)
            .unwrap()
            .rows
            .len(),
        1
    );
    let below = WireDecodeLimits {
        max_event_bytes: row.len() - 1,
        ..limits()
    };
    assert!(matches!(
        decode_batch_records(row.as_bytes(), &below),
        Err(BatchRecordsError::RowTooLarge { limit }) if limit == row.len() - 1
    ));
}

#[test]
fn item_limit_is_inclusive() {
    let content = "1\n2\n3\n";
    let at_limit = WireDecodeLimits {
        max_items: 3,
        ..limits()
    };
    assert_eq!(
        decode_batch_records(content.as_bytes(), &at_limit)
            .unwrap()
            .rows
            .len(),
        3
    );
    let below = WireDecodeLimits {
        max_items: 2,
        ..limits()
    };
    assert!(matches!(
        decode_batch_records(content.as_bytes(), &below),
        Err(BatchRecordsError::TooManyRows { limit: 2 })
    ));
}

#[test]
fn rejects_bytes_that_are_not_utf8() {
    assert!(matches!(
        decode_batch_records(b"{\"id\":\"\xff\"}\n", &limits()),
        Err(BatchRecordsError::NotUtf8(_))
    ));
}

#[rstest]
#[case::request(
    "{\"custom_id\":\"a\",\"url\":\"/v1/embeddings\",\"body\":{\"input\":\"x\"}}",
    true
)]
#[case::empty_body("{\"body\":{}}", true)]
#[case::body_string("{\"custom_id\":\"a\",\"body\":\"text\"}", false)]
#[case::body_null("{\"custom_id\":\"a\",\"body\":null}", false)]
#[case::body_array("{\"custom_id\":\"a\",\"body\":[]}", false)]
#[case::no_body("{\"custom_id\":\"a\"}", false)]
#[case::array("[{\"body\":{}}]", false)]
#[case::scalar("\"text\"", false)]
fn classifies_rows(#[case] text: &str, #[case] is_request: bool) {
    let value = val(text);
    let row = BatchRow::from(value.clone());
    assert_eq!(matches!(row, BatchRow::Request(_)), is_request);
    assert_eq!(serde_json::to_value(&row).unwrap(), value);
}

#[rstest]
#[case::replaces_model(
    "{\"custom_id\":\"a\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"alias\",\"n\":1}}",
    "{\"custom_id\":\"a\",\"method\":\"POST\",\"url\":\"/v1/chat/completions\",\"body\":{\"model\":\"deployment\",\"n\":1}}"
)]
#[case::adds_missing_model(
    "{\"custom_id\":7,\"body\":{\"input\":\"x\"}}",
    "{\"custom_id\":7,\"body\":{\"input\":\"x\",\"model\":\"deployment\"}}"
)]
#[case::leaves_string_body("{\"body\":\"text\"}", "{\"body\":\"text\"}")]
#[case::leaves_scalar("3", "3")]
fn rewrites_only_request_bodies(#[case] row: &str, #[case] expected: &str) {
    let rewritten = BatchRow::from(val(row)).with_model("deployment");
    assert_eq!(serde_json::to_value(rewritten).unwrap(), val(expected));
}

#[test]
fn create_request_round_trips_every_key() {
    let wire = val(
        "{\"completion_window\":\"24h\",\"endpoint\":\"/v1/chat/completions\",\"input_file_id\":\"file-1\",\
         \"metadata\":{\"team\":\"a\"},\"output_expires_after\":{\"anchor\":\"created_at\",\"seconds\":3600},\
         \"extra_headers\":{\"x-a\":\"b\"},\"extra_body\":{\"c\":\"d\"},\"timeout\":2.5}",
    );
    let request: CreateBatchRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(request.completion_window, Some(CompletionWindow::Day));
    assert_eq!(request.endpoint, Some(BatchEndpoint::ChatCompletions));
    assert_eq!(
        request.output_expires_after,
        Some(OutputExpiry {
            anchor: ExpiryAnchor::CreatedAt,
            seconds: 3600
        })
    );
    assert_eq!(request.timeout.map(f64::to_bits), Some(2.5_f64.to_bits()));
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
}

#[test]
fn create_request_keys_may_all_be_absent() {
    let request: CreateBatchRequest = serde_json::from_value(val("{}")).unwrap();
    assert_eq!(request, CreateBatchRequest::default());
    assert_eq!(serde_json::to_value(&request).unwrap(), val("{}"));
}

#[rstest]
#[case::completion_window("{\"completion_window\":\"48h\"}")]
#[case::endpoint("{\"endpoint\":\"/v1/unknown\"}")]
#[case::anchor("{\"output_expires_after\":{\"anchor\":\"now\",\"seconds\":1}}")]
#[case::metadata_value("{\"metadata\":{\"team\":1}}")]
fn create_request_rejects_values_outside_the_literals(#[case] wire: &str) {
    let error = serde_json::from_value::<CreateBatchRequest>(val(wire)).unwrap_err();
    assert!(!error.to_string().is_empty());
}

#[test]
fn litellm_create_request_adds_model_and_disable_fallbacks() {
    let wire = val("{\"input_file_id\":\"file-1\",\"model\":\"alias\",\"disable_fallbacks\":true}");
    let request: LiteLLMBatchCreateRequest = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(request.model.as_deref(), Some("alias"));
    assert_eq!(request.disable_fallbacks, Some(true));
    assert_eq!(request.input_file_id.as_deref(), Some("file-1"));
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
}

#[test]
fn retrieve_and_list_requests_read_their_keys() {
    let retrieve: BatchIdRequest =
        serde_json::from_value(val("{\"batch_id\":\"b-1\",\"timeout\":3.0}")).unwrap();
    assert_eq!(retrieve.batch_id.as_deref(), Some("b-1"));
    assert_eq!(retrieve.timeout.map(f64::to_bits), Some(3.0_f64.to_bits()));
    let list: ListBatchRequest = serde_json::from_value(val("{\"after\":\"b-1\"}")).unwrap();
    assert_eq!(list.after.as_deref(), Some("b-1"));
    let first_page: ListBatchRequest = serde_json::from_value(val("{}")).unwrap();
    assert_eq!(first_page.after, None);
    assert_eq!(serde_json::to_value(first_page).unwrap(), val("{}"));
}

#[rstest]
fn replays_corpus(#[files("fuzz/corpus/batch_records/*")] path: PathBuf) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let outcome = decode_batch_records(&fs::read(&path).unwrap(), &limits());
    if name.starts_with("rows-") {
        assert!(outcome.unwrap().unparsed.is_none(), "{name}");
    } else if name.starts_with("tail-") {
        assert!(outcome.unwrap().unparsed.is_some(), "{name}");
    } else if name == "not-utf8" {
        assert!(matches!(outcome, Err(BatchRecordsError::NotUtf8(_))));
    } else {
        assert!(outcome.unwrap().rows.is_empty(), "{name}");
    }
}

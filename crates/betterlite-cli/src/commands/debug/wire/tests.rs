#![cfg(test)]

use rstest::rstest;

use super::{RequestResponsePayload, SessionLogsPage, SpendLogRow};

fn row_with(fields: &str) -> SpendLogRow {
    serde_json::from_str(&format!(r#"{{"request_id":"req-1"{fields}}}"#)).unwrap()
}

#[rstest]
#[case::plain("0.000123", "0.000123")]
#[case::exponent("1.5e-05", "0.000015")]
#[case::capital_exponent("2E-3", "0.002")]
#[case::positive_exponent("1.5e2", "150")]
#[case::integer("7", "7")]
#[case::zero("0", "0")]
#[case::string("\"0.5\"", "0.5")]
#[case::beyond_float_precision("0.12345678901234567890", "0.12345678901234567890")]
fn spend_row_reads_the_spend_digits_exactly(#[case] lexeme: &str, #[case] expected: &str) {
    let row = row_with(&format!(r#","spend":{lexeme}"#));

    assert_eq!(row.spend.to_string(), expected);
}

#[rstest]
#[case::absent("")]
#[case::null(r#","spend":null"#)]
fn spend_row_defaults_the_spend_to_zero(#[case] fields: &str) {
    assert_eq!(row_with(fields).spend.to_string(), "0");
}

#[rstest]
#[case::word(r#","spend":"cheap""#)]
#[case::boolean(r#","spend":true"#)]
#[case::object(r#","spend":{}"#)]
fn spend_row_rejects_a_spend_that_is_not_a_number(#[case] fields: &str) {
    let outcome =
        serde_json::from_str::<SpendLogRow>(&format!(r#"{{"request_id":"req-1"{fields}}}"#));

    assert!(outcome.is_err());
}

#[test]
fn spend_row_requires_a_request_id() {
    assert!(serde_json::from_str::<SpendLogRow>(r#"{"spend":1}"#).is_err());
}

#[test]
fn spend_row_rejects_a_null_request_id() {
    assert!(serde_json::from_str::<SpendLogRow>(r#"{"request_id":null}"#).is_err());
}

#[rstest]
#[case::absent("")]
#[case::null(r#","prompt_tokens":null,"completion_tokens":null,"total_tokens":null"#)]
fn spend_row_defaults_the_token_counts_to_zero(#[case] fields: &str) {
    let row = row_with(fields);

    assert_eq!(
        (row.prompt_tokens, row.completion_tokens, row.total_tokens),
        (0, 0, 0)
    );
}

#[test]
fn spend_row_reads_the_token_counts() {
    let row = row_with(r#","prompt_tokens":11,"completion_tokens":22,"total_tokens":33"#);

    assert_eq!(
        (row.prompt_tokens, row.completion_tokens, row.total_tokens),
        (11, 22, 33)
    );
}

#[test]
fn spend_row_rejects_a_negative_token_count() {
    let outcome = serde_json::from_str::<SpendLogRow>(r#"{"request_id":"r","prompt_tokens":-1}"#);

    assert!(outcome.is_err());
}

#[test]
fn spend_row_reads_camel_case_times() {
    let row = row_with(r#","startTime":"2026-01-01T00:00:01","endTime":"2026-01-01T00:00:02""#);

    assert_eq!(row.start_time.as_deref(), Some("2026-01-01T00:00:01"));
    assert_eq!(row.end_time.as_deref(), Some("2026-01-01T00:00:02"));
}

#[test]
fn spend_row_reads_snake_case_times() {
    let row = row_with(r#","start_time":"a","end_time":"b""#);

    assert_eq!(row.start_time.as_deref(), Some("a"));
    assert_eq!(row.end_time.as_deref(), Some("b"));
}

#[rstest]
#[case::absent("")]
#[case::null(
    r#","start_time":null,"end_time":null,"model":null,"model_group":null,"custom_llm_provider":null,"api_base":null,"call_type":null,"status":null"#
)]
#[case::empty(
    r#","start_time":"","end_time":"","model":"","model_group":"","custom_llm_provider":"","api_base":"","call_type":"","status":"""#
)]
fn spend_row_treats_null_and_empty_text_as_absent(#[case] fields: &str) {
    let row = row_with(fields);
    let none: Option<String> = None;

    assert_eq!(
        [
            &row.start_time,
            &row.end_time,
            &row.model,
            &row.model_group,
            &row.custom_llm_provider,
            &row.api_base,
            &row.call_type,
            &row.status,
        ],
        [&none; 8]
    );
}

#[test]
fn spend_row_reads_the_text_fields() {
    let row = row_with(
        r#","model":"m","model_group":"g","custom_llm_provider":"p","api_base":"b","call_type":"c","status":"s""#,
    );

    assert_eq!(
        (
            row.model.as_deref(),
            row.model_group.as_deref(),
            row.custom_llm_provider.as_deref(),
            row.api_base.as_deref(),
            row.call_type.as_deref(),
            row.status.as_deref(),
        ),
        (
            Some("m"),
            Some("g"),
            Some("p"),
            Some("b"),
            Some("c"),
            Some("s")
        )
    );
}

#[rstest]
#[case::model_wins(r#","model":"m","model_group":"g""#, "m")]
#[case::group_when_model_empty(r#","model":"","model_group":"g""#, "g")]
#[case::group_when_model_absent(r#","model_group":"g""#, "g")]
#[case::unknown(r#","model":"","model_group":"""#, "?")]
fn spend_row_labels_the_model(#[case] fields: &str, #[case] expected: &str) {
    assert_eq!(row_with(fields).model_label(), expected);
}

#[rstest]
#[case::object(r#","metadata":{"status":"failure"}"#, Some("failure"))]
#[case::encoded_text(r#","metadata":"{\"status\":\"failure\"}""#, Some("failure"))]
#[case::null(r#","metadata":null"#, None)]
#[case::empty_text(r#","metadata":"""#, None)]
#[case::absent("", None)]
#[case::empty_object(r#","metadata":{}"#, None)]
fn spend_row_decodes_the_metadata_wire_shapes(
    #[case] fields: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(row_with(fields).metadata.status.as_deref(), expected);
}

#[rstest]
#[case::object(
    r#","metadata":{"error_information":{"error_code":"429","error_class":"RateLimit","error_message":"slow down","llm_provider":"openai"}}"#
)]
#[case::encoded_text(
    r#","metadata":"{\"error_information\":{\"error_code\":\"429\",\"error_class\":\"RateLimit\",\"error_message\":\"slow down\",\"llm_provider\":\"openai\"}}""#
)]
fn spend_row_decodes_the_error_information(#[case] fields: &str) {
    let row = row_with(fields);
    let error = row.metadata.error_information.unwrap();

    assert_eq!(
        (
            error.error_code.as_deref(),
            error.error_class.as_deref(),
            error.error_message.as_deref(),
            error.llm_provider.as_deref(),
        ),
        (
            Some("429"),
            Some("RateLimit"),
            Some("slow down"),
            Some("openai")
        )
    );
}

#[test]
fn spend_row_treats_empty_error_fields_as_absent() {
    let row = row_with(
        r#","metadata":{"error_information":{"error_code":"","error_class":null,"error_message":""}}"#,
    );
    let error = row.metadata.error_information.unwrap();

    assert_eq!(
        (
            error.error_code,
            error.error_class,
            error.error_message,
            error.llm_provider
        ),
        (None, None, None, None)
    );
}

#[test]
fn spend_row_rejects_metadata_text_that_is_not_json() {
    let outcome =
        serde_json::from_str::<SpendLogRow>(r#"{"request_id":"r","metadata":"not json"}"#);

    assert!(outcome.is_err());
}

#[rstest]
#[case::own_failure(r#","status":"failure""#, true)]
#[case::own_success(r#","status":"success""#, false)]
#[case::own_failure_over_metadata_success(
    r#","status":"failure","metadata":{"status":"success"}"#,
    true
)]
#[case::own_success_over_metadata_failure(
    r#","status":"success","metadata":{"status":"failure"}"#,
    false
)]
#[case::empty_status_falls_back(r#","status":"","metadata":{"status":"failure"}"#, true)]
#[case::null_status_falls_back(r#","status":null,"metadata":{"status":"failure"}"#, true)]
#[case::absent_status_falls_back(r#","metadata":{"status":"failure"}"#, true)]
#[case::encoded_metadata_falls_back(r#","metadata":"{\"status\":\"failure\"}""#, true)]
#[case::both_empty(r#","status":"","metadata":{"status":""}"#, false)]
#[case::both_absent("", false)]
#[case::other_word(r#","status":"error""#, false)]
fn spend_row_fails_by_its_own_status_then_the_metadata_status(
    #[case] fields: &str,
    #[case] expected: bool,
) {
    assert_eq!(row_with(fields).failed(), expected);
}

#[test]
fn spend_row_reads_a_session_page() {
    let page: SessionLogsPage = serde_json::from_str(
        r#"{"data":[{"request_id":"a"},{"request_id":"b"}],"total":7,"total_pages":2}"#,
    )
    .unwrap();

    assert_eq!((page.data.len(), page.total, page.total_pages), (2, 7, 2));
}

#[rstest]
#[case::no_data(r#"{"total":1,"total_pages":1}"#)]
#[case::no_total(r#"{"data":[],"total_pages":1}"#)]
#[case::no_total_pages(r#"{"data":[],"total":1}"#)]
#[case::bad_row(r#"{"data":[{"spend":1}],"total":1,"total_pages":1}"#)]
fn spend_row_rejects_an_incomplete_session_page(#[case] body: &str) {
    assert!(serde_json::from_str::<SessionLogsPage>(body).is_err());
}

#[rstest]
#[case::string(r#"{"proxy_server_request":"plain text"}"#, "plain text")]
#[case::object(r#"{"proxy_server_request":{"a":1}}"#, "{\n  \"a\": 1\n}")]
#[case::list(r#"{"proxy_server_request":[1]}"#, "[\n  1\n]")]
#[case::number(r#"{"proxy_server_request":4}"#, "4")]
#[case::null(r#"{"proxy_server_request":null}"#, "null")]
#[case::absent("{}", "null")]
fn spend_row_renders_a_payload_field_as_json_text(#[case] body: &str, #[case] expected: &str) {
    let payload: RequestResponsePayload = serde_json::from_str(body).unwrap();

    assert_eq!(payload.proxy_server_request.text(), expected);
}

#[test]
fn spend_row_reads_every_payload_field() {
    let payload: RequestResponsePayload = serde_json::from_str(
        r#"{"proxy_server_request":"req","response":"resp","messages":"msgs","extra":1}"#,
    )
    .unwrap();

    assert_eq!(
        (
            payload.proxy_server_request.text(),
            payload.response.text(),
            payload.messages.text()
        ),
        ("req".to_owned(), "resp".to_owned(), "msgs".to_owned())
    );
}

#[test]
fn spend_row_reads_a_null_payload_as_none() {
    let payload: Option<RequestResponsePayload> = serde_json::from_str("null").unwrap();

    assert!(payload.is_none());
}

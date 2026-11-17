#![cfg(test)]

use betterlite_models::decisions::Json;
use http::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use crate::vertex_batch_download::policy::{DownloadPolicy, download_policy};
use crate::vertex_batch_download::records::{
    ContentResponse, ConversionContext, transform_file_content_response,
};

const NOW: i64 = 1_700_000_000;
const URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-2.5-flash%2Fbatch.jsonl";
const PASSTHROUGH_URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpassthrough%2Fbatch.jsonl";
const ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":1,"totalTokenCount":2,"trafficType":"ON_DEMAND"}}}"#;
const OTHER_ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"candidates":[{"content":{"role":"model","parts":[{"text":"yo"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":2,"totalTokenCount":4}}}"#;
const STATUS_ROW: &str =
    r#"{"request":{},"status":"FAILED_PRECONDITION","processed_time":"","response":{}}"#;
const BLOCKED_ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"promptFeedback":{"blockReason":"SAFETY"},"usageMetadata":{"promptTokenCount":4}}}"#;
const NO_USAGE_ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{"candidates":[{"content":{"role":"model","parts":[{"text":"hi"}]},"finishReason":"STOP"}]}}"#;
const OTHER_KIND_ROW: &str = r#"{"request":{},"status":"","processed_time":"","response":{}}"#;
const EMBEDDING_ROWS: &str = concat!(
    r#"{"key":"id%23one#1/2","request":{"content":{}},"status":"","response":{"embedding":{"values":[2]},"usageMetadata":{"promptTokenCount":2}}}"#,
    "\n",
    r#"{"key":"id%23one#0/2","request":{"content":{}},"status":"","response":{"embedding":{"values":[1]},"usageMetadata":{"promptTokenCount":1}}}"#,
);
const EMBEDDING_GROUPS: &str = concat!(
    r#"{"key":"id%23one#0/1","request":{"content":{}},"status":"","response":{"embedding":{"values":[1]},"usageMetadata":{"promptTokenCount":1}}}"#,
    "\n",
    r#"{"key":"id%23two#0/1","request":{"content":{}},"status":"","response":{"embedding":{"values":[2]},"usageMetadata":{"promptTokenCount":2}}}"#,
);

fn context() -> ConversionContext {
    let mut issued = 0_u128;
    ConversionContext {
        now: NOW,
        ids: Box::new(move || {
            issued = issued.saturating_add(1);
            Uuid::from_u128(issued)
        }),
    }
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn field<'value>(value: &'value Json, name: &str) -> &'value Json {
    let Json::Object(fields) = value else {
        panic!("{name} is not read from an object");
    };
    fields.get(name).unwrap()
}

fn keys(value: &Json) -> Vec<&str> {
    let Json::Object(fields) = value else {
        panic!("the keys of a non-object were asked for");
    };
    let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    names
}

fn declared_length(response: &ContentResponse) -> usize {
    response
        .headers
        .get(CONTENT_LENGTH)
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

fn rows(response: &ContentResponse) -> Vec<Json> {
    let text = String::from_utf8(response.content.clone()).unwrap();
    text.split('\n').map(json).collect()
}

// a converted line is an OpenAI batch row, and the completion sits under response.body
fn completion(row: &Json) -> &Json {
    field(field(row, "response"), "body")
}

fn convert(content: &str, url: &str) -> ContentResponse {
    let mut state = context();
    let length = content.len().to_string();
    transform_file_content_response(
        content.as_bytes(),
        url,
        &headers(&[
            (CONTENT_TYPE.as_str(), "application/json"),
            (CONTENT_LENGTH.as_str(), &length),
        ]),
        false,
        &mut state,
    )
}

fn assert_custom_id(row: &str, expected: &str) {
    let converted = convert(row, URL);
    let body = json(&String::from_utf8(converted.content).unwrap());
    assert_eq!(
        field(&body, "custom_id"),
        &Json::Text(String::from(expected))
    );
}

#[test]
fn download_policy_covers_the_switch_and_the_passthrough_prefix() {
    assert_eq!(download_policy(true, URL), DownloadPolicy::Original);
    assert_eq!(
        download_policy(false, PASSTHROUGH_URL),
        DownloadPolicy::Original
    );
    assert_eq!(download_policy(false, URL), DownloadPolicy::Convert);
}

#[test]
fn disabled_returns_the_original_bytes() {
    let mut state = context();
    let length = ROW.len().to_string();
    let original = headers(&[
        (CONTENT_TYPE.as_str(), "application/json"),
        (CONTENT_LENGTH.as_str(), &length),
    ]);
    let response =
        transform_file_content_response(ROW.as_bytes(), URL, &original, true, &mut state);
    assert_eq!(response.content, ROW.as_bytes());
    assert_eq!(response.headers, original);
}

#[test]
fn passthrough_urls_keep_the_original_bytes() {
    let response = convert(ROW, PASSTHROUGH_URL);
    assert_eq!(response.content, ROW.as_bytes());
    assert_eq!(declared_length(&response), ROW.len());
}

#[test]
fn an_empty_download_is_returned_untouched() {
    let mut state = context();
    let original = headers(&[(CONTENT_LENGTH.as_str(), "0")]);
    let response = transform_file_content_response(b"", URL, &original, false, &mut state);
    assert!(response.content.is_empty());
    assert_eq!(response.headers, original);
}

#[test]
fn unknown_formats_pass_through_exactly() {
    let binary: &[u8] = b"\x00\x01{\"not\":json}\xff\n\x02";
    let mut state = context();
    let length = binary.len().to_string();
    let original = headers(&[
        (CONTENT_TYPE.as_str(), "application/octet-stream"),
        (CONTENT_LENGTH.as_str(), &length),
    ]);
    let response =
        transform_file_content_response(binary, "https://x/y", &original, false, &mut state);
    assert_eq!(response.content, binary);
    assert_eq!(response.headers, original);
}

#[test]
fn a_generation_row_becomes_a_chat_completion() {
    let response = convert(ROW, URL);
    let row = json(&String::from_utf8(response.content.clone()).unwrap());
    assert_eq!(
        field(&row, "id"),
        &json(r#""batch_req_00000000-0000-0000-0000-000000000002""#)
    );
    assert_eq!(field(&row, "custom_id"), &json(r#""unknown""#));
    assert_eq!(field(&row, "error"), &Json::Null);
    let body = completion(&row);
    assert_eq!(
        field(body, "id"),
        &json(r#""chatcmpl-00000000-0000-0000-0000-000000000001""#)
    );
    assert_eq!(field(body, "created"), &json("1700000000"));
    assert_eq!(field(body, "model"), &json(r#""gemini-1.5-flash-001""#));
    assert_eq!(field(body, "object"), &json(r#""chat.completion""#));
    assert_eq!(field(body, "system_fingerprint"), &Json::Null);
    let Json::List(choices) = field(body, "choices") else {
        panic!("choices is not a list");
    };
    assert_eq!(choices.len(), 1);
    assert_eq!(field(&choices[0], "finish_reason"), &json(r#""stop""#));
    assert_eq!(field(&choices[0], "index"), &json("0"));
    assert_eq!(declared_length(&response), response.content.len());
    assert_eq!(
        response.headers.get(CONTENT_TYPE).unwrap(),
        "application/json"
    );
}

#[test]
fn a_generation_row_carries_the_message_fields() {
    let response = convert(ROW, URL);
    let row = json(&String::from_utf8(response.content).unwrap());
    let Json::List(choices) = field(completion(&row), "choices") else {
        panic!("choices is not a list");
    };
    assert_eq!(
        keys(field(&choices[0], "provider_specific_fields")),
        ["native_finish_reason"]
    );
    let message = field(&choices[0], "message");
    assert_eq!(
        keys(message),
        [
            "content",
            "function_call",
            "images",
            "role",
            "thinking_blocks",
            "tool_calls"
        ]
    );
    assert_eq!(field(message, "content"), &json(r#""hi""#));
    assert_eq!(field(message, "role"), &json(r#""assistant""#));
}

#[test]
fn a_generation_row_carries_the_usage_and_metadata() {
    let response = convert(ROW, URL);
    let row = json(&String::from_utf8(response.content).unwrap());
    let body = completion(&row);
    let usage = field(body, "usage");
    assert_eq!(field(usage, "prompt_tokens"), &json("1"));
    assert_eq!(field(usage, "completion_tokens"), &json("1"));
    assert_eq!(field(usage, "total_tokens"), &json("2"));
    assert_eq!(
        field(field(usage, "completion_tokens_details"), "text_tokens"),
        &json("1")
    );
    assert_eq!(
        keys(field(usage, "prompt_tokens_details")),
        [
            "audio_tokens",
            "cached_tokens",
            "image_tokens",
            "text_tokens",
            "video_tokens"
        ]
    );
    assert_eq!(field(usage, "cache_read_input_tokens"), &Json::Null);
    assert_eq!(
        field(body, "vertex_ai_grounding_metadata"),
        &Json::List(Vec::new())
    );
    assert_eq!(
        field(body, "vertex_ai_url_context_metadata"),
        &Json::List(Vec::new())
    );
    assert_eq!(
        field(body, "vertex_ai_safety_results"),
        &Json::List(Vec::new())
    );
    assert_eq!(
        field(body, "vertex_ai_citation_metadata"),
        &Json::List(Vec::new())
    );
}

#[test]
fn a_present_model_version_wins_and_loses_its_suffix() {
    let row = ROW.replace(
        r#""candidates""#,
        r#""modelVersion":"gemini-2.5-pro@001","candidates""#,
    );
    let response = convert(&row, URL);
    let converted = json(&String::from_utf8(response.content).unwrap());
    assert_eq!(
        field(completion(&converted), "model"),
        &json(r#""gemini-2.5-pro""#)
    );
}

#[test]
fn a_null_model_version_returns_the_whole_payload() {
    let row = ROW.replace(r#""candidates""#, r#""modelVersion":null,"candidates""#);
    let response = convert(&row, URL);
    assert_eq!(response.content, row.as_bytes());
    assert_eq!(declared_length(&response), row.len());
}

#[test]
fn a_provider_status_becomes_a_vertex_ai_error_row() {
    let response = convert(STATUS_ROW, URL);
    let body = json(&String::from_utf8(response.content.clone()).unwrap());
    assert_eq!(
        field(&body, "id"),
        &json(r#""batch_req_00000000-0000-0000-0000-000000000001""#)
    );
    assert_eq!(field(&body, "custom_id"), &json(r#""unknown""#));
    assert_eq!(field(&body, "response"), &Json::Null);
    assert_eq!(
        *field(&body, "error"),
        json(r#"{"code":"vertex_ai_error","message":"FAILED_PRECONDITION"}"#)
    );
    assert_eq!(declared_length(&response), response.content.len());
}

#[test]
fn a_blocked_prompt_keeps_the_raw_counters() {
    let response = convert(BLOCKED_ROW, URL);
    let row = json(&String::from_utf8(response.content).unwrap());
    let body = completion(&row);
    let Json::List(choices) = field(body, "choices") else {
        panic!("choices is not a list");
    };
    assert_eq!(
        field(&choices[0], "finish_reason"),
        &json(r#""content_filter""#)
    );
    let message = field(&choices[0], "message");
    assert_eq!(
        keys(message),
        ["content", "function_call", "role", "tool_calls"]
    );
    assert_eq!(field(message, "content"), &Json::Null);
    let usage = field(body, "usage");
    assert_eq!(field(usage, "prompt_tokens"), &json("4"));
    assert_eq!(field(usage, "completion_tokens"), &json("0"));
    assert_eq!(field(usage, "prompt_tokens_details"), &Json::Null);
    assert_eq!(field(usage, "completion_tokens_details"), &Json::Null);
    assert!(!keys(usage).contains(&"cache_read_input_tokens"));
    assert!(!keys(body).contains(&"vertex_ai_grounding_metadata"));
}

#[test]
fn a_failed_conversion_becomes_a_transformation_error_row() {
    let response = convert(NO_USAGE_ROW, URL);
    let body = json(&String::from_utf8(response.content.clone()).unwrap());
    assert_eq!(
        field(&body, "id"),
        &json(r#""batch_req_00000000-0000-0000-0000-000000000002""#)
    );
    assert_eq!(field(&body, "response"), &Json::Null);
    let error = field(&body, "error");
    assert_eq!(field(error, "code"), &json(r#""transformation_error""#));
    let Json::Text(message) = field(error, "message") else {
        panic!("the transformation message is not text");
    };
    assert!(message.starts_with("Failed to transform response: "));
    assert!(message.contains("usageMetadata"));
    assert_eq!(declared_length(&response), response.content.len());
}

#[test]
fn a_row_of_neither_output_kind_returns_the_whole_payload() {
    let response = convert(OTHER_KIND_ROW, URL);
    assert_eq!(response.content, OTHER_KIND_ROW.as_bytes());
    assert_eq!(declared_length(&response), OTHER_KIND_ROW.len());
}

#[test]
fn a_later_unparsable_row_returns_the_whole_payload() {
    let content = format!("{ROW}\nnot json\n{ROW}");
    let response = convert(&content, URL);
    assert_eq!(response.content, content.as_bytes());
    assert_eq!(declared_length(&response), content.len());
}

#[test]
fn blank_lines_between_rows_are_skipped() {
    let content = format!("{ROW}\n\n  \n{OTHER_ROW}\n");
    let response = convert(&content, URL);
    let body = rows(&response);
    assert_eq!(body.len(), 2);
    assert_eq!(field(completion(&body[1]), "created"), &json("1700000000"));
}

// str::trim follows the White_Space property and keeps U+001C, which Python's str.strip drops
#[test]
fn a_file_separator_prefix_returns_the_whole_payload() {
    let content = format!("\u{1c}{ROW}\n");
    let response = convert(&content, URL);
    assert_eq!(response.content, content.as_bytes());
    assert_eq!(declared_length(&response), content.len());
}

#[test]
fn generation_rows_are_newline_joined() {
    let content = format!("{ROW}\n{OTHER_ROW}");
    let response = convert(&content, URL);
    let body = rows(&response);
    assert_eq!(body.len(), 2);
    assert_eq!(
        field(&body[1], "id"),
        &json(r#""batch_req_00000000-0000-0000-0000-000000000004""#)
    );
    assert_eq!(
        field(completion(&body[1]), "id"),
        &json(r#""chatcmpl-00000000-0000-0000-0000-000000000003""#)
    );
    assert_eq!(
        field(field(&body[1], "response"), "request_id"),
        &json(r#""chatcmpl-00000000-0000-0000-0000-000000000003""#)
    );
    assert_eq!(declared_length(&response), response.content.len());
}

#[test]
fn an_invalid_utf8_row_returns_the_whole_payload() {
    let mut content = ROW.as_bytes().to_vec();
    content.push(b'\n');
    content.extend_from_slice(b"\xff\xfe\n");
    let mut state = context();
    let length = content.len().to_string();
    let original = headers(&[(CONTENT_LENGTH.as_str(), &length)]);
    let response = transform_file_content_response(&content, URL, &original, false, &mut state);
    assert_eq!(response.content, content);
    assert_eq!(response.headers, original);
}

#[test]
fn embeddings_regroup_and_recalculate_content_length() {
    let response = convert(EMBEDDING_ROWS, URL);
    let body = json(&String::from_utf8(response.content.clone()).unwrap());
    assert_eq!(
        field(&body, "id"),
        &json(r#""batch_req_00000000-0000-0000-0000-000000000001""#)
    );
    assert_eq!(field(&body, "custom_id"), &json(r#""id#one""#));
    let Json::List(records) = field(field(field(&body, "response"), "body"), "data") else {
        panic!("data is not a list");
    };
    assert_eq!(records.len(), 2);
    assert_eq!(field(&records[0], "index"), &json("0"));
    assert_eq!(field(&records[0], "embedding"), &json("[1]"));
    assert_eq!(field(&records[1], "index"), &json("1"));
    assert_eq!(field(&records[1], "embedding"), &json("[2]"));
    assert_eq!(declared_length(&response), response.content.len());
    assert_ne!(declared_length(&response), EMBEDDING_ROWS.len());
}

#[test]
fn embedding_groups_are_newline_joined() {
    let response = convert(EMBEDDING_GROUPS, URL);
    let body = rows(&response);
    assert_eq!(body.len(), 2);
    let Json::Text(first) = field(&body[0], "custom_id") else {
        panic!("custom_id is not text");
    };
    let Json::Text(second) = field(&body[1], "custom_id") else {
        panic!("custom_id is not text");
    };
    assert_eq!([first.as_str(), second.as_str()], ["id#one", "id#two"]);
    assert_eq!(declared_length(&response), response.content.len());
}

#[test]
fn keys_and_labels_correlate_rows() {
    let keyed = ROW.replace(r#""request":{}"#, r#""key":"req%23one#0/1","request":{}"#);
    assert_custom_id(&keyed, "req#one#0/1");
    let labelled = ROW.replace(
        r#""request":{}"#,
        r#""request":{"labels":{"litellm_custom_id":"req-1"}}"#,
    );
    assert_custom_id(&labelled, "req-1");
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::iter::repeat_n;
use std::path::PathBuf;

use rstest::rstest;
use tokio::runtime::Builder;

use crate::arguments::json::Json;
use crate::gemini_tool_invokes::limits::{MAX_MESSAGE_BYTES, MAX_TOOL_CALLS};
use crate::gemini_tool_results::convert::convert_tool_result_to_gemini;
use crate::gemini_tool_results::error::GeminiToolResultsError;
use crate::gemini_tool_results::parser::parse_tool_result_message;
use crate::gemini_tool_results::types::{GeminiToolResult, LastToolCallMessage};
use crate::media::inline::RemoteMediaFetcher;
use crate::media::limits::MAX_MESSAGE_PARTS;
use crate::media_fetch::error::ImageFetchError;
use crate::message_content::Message;

struct MapSource<'urls> {
    data_urls: &'urls BTreeMap<String, String>,
}

impl MapSource<'_> {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for MapSource<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.result_for(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.iter().map(|url| self.result_for(url)).collect())
    }
}

struct Call {
    message: &'static str,
    last: Option<&'static str>,
    forward: bool,
    urls: &'static [(&'static str, &'static str)],
}

struct ConvertCase {
    call: Call,
    expected: &'static str,
}

struct FailureCase {
    call: Call,
    failure: Failure,
}

struct PreservedCase {
    call: Call,
    expected: &'static str,
}

enum Failure {
    Role,
    MissingName,
    Field(&'static str),
}

enum Scenario {
    Convert(ConvertCase),
    Failure(FailureCase),
    Preserved(PreservedCase),
    ParseFailure(&'static str),
    OversizedMessage,
    MessageAtByteLimit,
    TooManyParts,
    MaxParts,
    TooManyToolCalls,
    MaxToolCalls,
}

fn parsed_json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn parse_message(text: &str) -> Message {
    parse_tool_result_message(text.as_bytes()).unwrap()
}

async fn converted(call: &Call) -> Result<GeminiToolResult, GeminiToolResultsError> {
    let message = parse_message(call.message);
    let last = call
        .last
        .map(|text| serde_json::from_str::<LastToolCallMessage>(text).unwrap());
    let data_urls = call
        .urls
        .iter()
        .map(|(url, data)| ((*url).to_owned(), (*data).to_owned()))
        .collect::<BTreeMap<String, String>>();
    let source = MapSource {
        data_urls: &data_urls,
    };
    convert_tool_result_to_gemini(&message, last.as_ref(), call.forward, &source).await
}

async fn assert_converted(case: &ConvertCase) {
    let converted = converted(&case.call).await.unwrap();
    assert_eq!(
        serde_json::to_value(&converted).unwrap(),
        serde_json::to_value(parsed_json(case.expected)).unwrap()
    );
}

async fn assert_failure_case(case: &FailureCase) {
    let error = converted(&case.call).await.unwrap_err();
    match case.failure {
        Failure::Role => assert!(matches!(&error, GeminiToolResultsError::Role { .. })),
        Failure::MissingName => {
            assert!(matches!(&error, GeminiToolResultsError::MissingName));
        }
        Failure::Field(path) => {
            assert!(matches!(&error, GeminiToolResultsError::Field { .. }));
            assert!(
                error.to_string().contains(path),
                "{error} does not name {path}"
            );
        }
    }
}

async fn assert_preserved(case: &PreservedCase) {
    let message = parse_message(case.call.message);
    let untouched = message.clone();
    let last = case
        .call
        .last
        .map(|text| serde_json::from_str::<LastToolCallMessage>(text).unwrap());
    let data_urls = case
        .call
        .urls
        .iter()
        .map(|(url, data)| ((*url).to_owned(), (*data).to_owned()))
        .collect::<BTreeMap<String, String>>();
    let source = MapSource {
        data_urls: &data_urls,
    };
    let converted =
        convert_tool_result_to_gemini(&message, last.as_ref(), case.call.forward, &source)
            .await
            .unwrap();
    drop(convert_tool_result_to_gemini(&message, last.as_ref(), !case.call.forward, &source).await);
    assert_eq!(message, untouched);
    assert_eq!(
        serde_json::to_value(&converted).unwrap(),
        serde_json::to_value(parsed_json(case.expected)).unwrap()
    );
}

fn assert_parse_failure(text: &str) {
    let error = parse_tool_result_message(text.as_bytes()).unwrap_err();
    assert!(matches!(
        &error,
        GeminiToolResultsError::MessageDecode { .. }
    ));
}

fn assert_oversized_message() {
    let bytes = vec![b' '; MAX_MESSAGE_BYTES.saturating_add(1)];
    let error = parse_tool_result_message(&bytes).unwrap_err();
    assert!(matches!(
        &error,
        GeminiToolResultsError::MessageTooLarge { length, limit }
            if *length == MAX_MESSAGE_BYTES.saturating_add(1) && *limit == MAX_MESSAGE_BYTES
    ));
}

fn assert_message_at_byte_limit() {
    let mut bytes = br#"{"role":"tool","tool_call_id":"c","name":"n"}"#.to_vec();
    bytes.resize(MAX_MESSAGE_BYTES, b' ');
    let message = parse_tool_result_message(&bytes).expect("a message at the byte limit parses");
    assert_eq!(message.role, "tool");
}

fn text_parts_message(count: usize) -> String {
    let parts = repeat_n(r#"{"type":"text","text":"a"}"#, count)
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"role":"tool","tool_call_id":"call","name":"n","content":[{parts}]}}"#)
}

async fn bound_result(
    message: &Message,
    last: Option<&LastToolCallMessage>,
) -> Result<GeminiToolResult, GeminiToolResultsError> {
    let data_urls = BTreeMap::new();
    let source = MapSource {
        data_urls: &data_urls,
    };
    convert_tool_result_to_gemini(message, last, false, &source).await
}

fn tool_calls_history(count: usize) -> String {
    let tool = r#"{"id":"call","function":{"name":"n","arguments":"{}"}}"#;
    let calls = repeat_n(tool, count).collect::<Vec<_>>().join(",");
    format!(r#"{{"tool_calls":[{calls}]}}"#)
}

async fn assert_too_many_parts() {
    let message = parse_message(&text_parts_message(MAX_MESSAGE_PARTS.saturating_add(1)));
    let error = bound_result(&message, None).await.unwrap_err();
    let GeminiToolResultsError::TooManyParts { limit } = error else {
        panic!("a long content list is refused");
    };
    assert_eq!(limit, MAX_MESSAGE_PARTS);
}

async fn assert_max_parts() {
    let message = parse_message(&text_parts_message(MAX_MESSAGE_PARTS));
    let result = bound_result(&message, None).await.unwrap();
    let GeminiToolResult::Single(part) = result else {
        panic!("a text-only result is a single part");
    };
    let Json::Object(response) = part.function_response.response else {
        panic!("the response is an object");
    };
    assert_eq!(
        response.get("content").and_then(Json::as_str).map(str::len),
        Some(MAX_MESSAGE_PARTS)
    );
}

async fn assert_too_many_tool_calls() {
    let last = serde_json::from_str::<LastToolCallMessage>(&tool_calls_history(
        MAX_TOOL_CALLS.saturating_add(1),
    ))
    .unwrap();
    let message = parse_message(r#"{"role":"tool","tool_call_id":"call","content":"x"}"#);
    let error = bound_result(&message, Some(&last)).await.unwrap_err();
    assert!(matches!(
        &error,
        GeminiToolResultsError::TooManyToolCalls { limit } if *limit == MAX_TOOL_CALLS
    ));
}

async fn assert_max_tool_calls() {
    let last =
        serde_json::from_str::<LastToolCallMessage>(&tool_calls_history(MAX_TOOL_CALLS)).unwrap();
    let message = parse_message(r#"{"role":"tool","tool_call_id":"call","content":"x"}"#);
    let result = bound_result(&message, Some(&last)).await.unwrap();
    let GeminiToolResult::Single(part) = result else {
        panic!("a text-only result is a single part");
    };
    assert_eq!(part.function_response.name, "n");
}

async fn run(scenario: Scenario) {
    match scenario {
        Scenario::Convert(case) => assert_converted(&case).await,
        Scenario::Failure(case) => assert_failure_case(&case).await,
        Scenario::Preserved(case) => assert_preserved(&case).await,
        Scenario::ParseFailure(text) => assert_parse_failure(text),
        Scenario::OversizedMessage => assert_oversized_message(),
        Scenario::MessageAtByteLimit => assert_message_at_byte_limit(),
        Scenario::TooManyParts => assert_too_many_parts().await,
        Scenario::MaxParts => assert_max_parts().await,
        Scenario::TooManyToolCalls => assert_too_many_tool_calls().await,
        Scenario::MaxToolCalls => assert_max_tool_calls().await,
    }
}

#[rstest]
#[case::plain_text_content_becomes_the_response_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_1","name":"get_weather","content":"sunny"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"get_weather","response":{"content":"sunny"}}}"#,
}))]
#[case::function_role_name_is_used(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"function","name":"get_weather","content":"sunny"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"get_weather","response":{"content":"sunny"}}}"#,
}))]
#[case::absent_content_becomes_an_empty_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_1","name":"n"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::null_content_becomes_an_empty_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_1","name":"n","content":null}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::empty_content_string_becomes_empty_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_1","name":"n","content":""}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::json_object_content_is_the_response(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"{\"temperature\":22.5}"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"temperature":22.5}}}"#,
}))]
#[case::leading_whitespace_before_a_json_object_is_ignored(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"  {\"a\":1}"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"a":1}}}"#,
}))]
#[case::json_list_content_is_wrapped_as_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"[1,2]"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"[1,2]"}}}"#,
}))]
#[case::json_null_content_is_wrapped_as_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"null"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"null"}}}"#,
}))]
#[case::invalid_json_content_is_wrapped_as_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"{\"a\":"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"{\"a\":"}}}"#,
}))]
#[case::json_with_trailing_text_is_wrapped_as_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"{\"a\":1} x"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"{\"a\":1} x"}}}"#,
}))]
#[case::text_parts_are_concatenated(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"text","text":"one "},{"type":"text","text":"two"}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"one two"}}}"#,
}))]
#[case::a_missing_text_part_adds_nothing(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"text"}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::a_null_text_part_adds_nothing(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"text","text":null}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::unknown_content_parts_are_skipped(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"tool_reference","tool_name":"x"},{"type":"text","text":"kept"}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"kept"}}}"#,
}))]
#[case::an_empty_content_list_is_empty_content(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::a_data_url_string_is_promoted_to_inline_data(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"data:image/png;base64,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::an_uppercase_data_prefix_is_promoted(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"DATA:image/png;base64,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::data_url_mime_parameters_are_stripped(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"data:image/png;charset=UTF-8;base64,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::data_url_mime_whitespace_is_trimmed(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"data:image/png ;base64,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::an_empty_data_url_payload_still_yields_inline_data(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"data:image/png;base64,"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"","mime_type":"image/png"}}]}}]"#,
}))]
#[case::a_non_image_data_url_stays_text(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"data:application/pdf;base64,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"data:application/pdf;base64,QUJD"}}}"#,
}))]
#[case::a_data_url_without_the_marker_stays_text(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"data:image/png,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"data:image/png,QUJD"}}}"#,
}))]
#[case::a_mistyped_data_prefix_stays_text(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"dat:image/png;base64,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"dat:image/png;base64,QUJD"}}}"#,
}))]
#[case::an_uppercase_mime_is_not_promoted(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":"data:IMAGE/PNG;base64,QUJD"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"data:IMAGE/PNG;base64,QUJD"}}}"#,
}))]
#[case::an_anthropic_image_block_becomes_inline_data(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image","source":{"type":"base64","data":"QUJD","media_type":"image/webp"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/webp"}}]}}]"#,
}))]
#[case::an_anthropic_image_block_defaults_the_media_type(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image","source":{"type":"base64","data":"QUJD"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/jpeg"}}]}}]"#,
}))]
#[case::an_image_block_without_a_base64_source_is_skipped(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image","source":{"type":"url","url":"http://x/a.png"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::an_input_image_data_url_becomes_inline_data(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"text","text":"shot:"},{"type":"input_image","image_url":{"url":"data:image/png;base64,QUJD"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":"shot:"},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::an_image_url_string_is_accepted(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image_url","image_url":"data:image/png;base64,QUJD"}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::an_image_url_object_without_a_url_is_skipped(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"input_image","image_url":{}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::a_null_image_url_is_skipped(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"input_image","image_url":null}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::an_http_image_url_is_downloaded(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"input_image","image_url":{"url":"http://example.test/a.png"}}]}"#,
        last: None,
        forward: false,
        urls: &[("http://example.test/a.png", "data:image/png;base64,QUJD")],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::a_failed_download_drops_the_media(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"input_image","image_url":{"url":"http://example.test/a.png"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::a_malformed_image_url_drops_the_media(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image_url","image_url":"image/png;base64,AA"}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::a_file_block_uses_its_file_data(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file_data":"data:image/png;base64,QUJD"}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::a_file_block_reads_the_nested_file_data(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"input_file","file":{"file_data":"data:application/pdf;base64,QUJD"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"application/pdf"}}]}}]"#,
}))]
#[case::a_file_block_accepts_a_string_file(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file":"data:image/png;base64,QUJD"}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::a_file_block_without_data_is_skipped(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file":{"file_id":"file-1"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":""}}}"#,
}))]
#[case::empty_top_level_file_data_falls_back_to_the_file(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file_data":"","file":{"file_data":"data:image/png;base64,QUJD"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QUJD","mime_type":"image/png"}}]}}]"#,
}))]
#[case::top_level_file_data_wins_over_the_file(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file_data":"data:image/png;base64,QQ","file":{"file_data":"data:image/png;base64,QUJD"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QQ","mime_type":"image/png"}}]}}]"#,
}))]
#[case::several_media_parts_keep_their_order(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image","source":{"type":"base64","data":"QQ","media_type":"image/png"}},{"type":"image","source":{"type":"base64","data":"Qg","media_type":"image/webp"}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":""},"parts":[{"inline_data":{"data":"QQ","mime_type":"image/png"}},{"inline_data":{"data":"Qg","mime_type":"image/webp"}}]}}]"#,
}))]
#[case::the_name_is_recovered_from_parallel_tool_calls(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_b","content":"x"}"#,
        last: Some(r#"{"role":"assistant","tool_calls":[{"id":"call_a","function":{"name":"first","arguments":"{}"}},{"id":"call_b","function":{"name":"second","arguments":"{}"}}]}"#),
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"second","response":{"content":"x"}}}"#,
}))]
#[case::an_unmatched_id_keeps_the_message_name(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_c","name":"given","content":"x"}"#,
        last: Some(r#"{"tool_calls":[{"id":"call_a","function":{"name":"first","arguments":"{}"}}]}"#),
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"given","response":{"content":"x"}}}"#,
}))]
#[case::the_last_matching_duplicate_wins(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_b","content":"x"}"#,
        last: Some(r#"{"tool_calls":[{"id":"call_b","function":{"name":"first","arguments":"{}"}},{"id":"call_b","function":{"name":"second","arguments":"{}"}}]}"#),
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"second","response":{"content":"x"}}}"#,
}))]
#[case::an_empty_tool_call_id_never_matches(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"","name":"given","content":"x"}"#,
        last: Some(r#"{"tool_calls":[{"id":"","function":{"name":"second","arguments":"{}"}}]}"#),
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"given","response":{"content":"x"}}}"#,
}))]
#[case::null_tool_calls_use_the_message_name(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_a","name":"given","content":"x"}"#,
        last: Some(r#"{"tool_calls":null}"#),
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"given","response":{"content":"x"}}}"#,
}))]
#[case::a_function_call_id_is_forwarded(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_abc","name":"n","content":"x"}"#,
        last: None,
        forward: true,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"x"},"id":"call_abc"}}"#,
}))]
#[case::forwarding_strips_the_thought_signature_suffix(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_abc__thought__sig","name":"n","content":"x"}"#,
        last: None,
        forward: true,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"x"},"id":"call_abc"}}"#,
}))]
#[case::forwarding_strips_at_the_first_separator(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call__thought__a__thought__b","name":"n","content":"x"}"#,
        last: None,
        forward: true,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"x"},"id":"call"}}"#,
}))]
#[case::forwarding_off_omits_the_id(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_abc","name":"n","content":"x"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"x"}}}"#,
}))]
#[case::a_signature_only_id_is_not_forwarded(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"__thought__sig","name":"n","content":"x"}"#,
        last: None,
        forward: true,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"x"}}}"#,
}))]
#[case::an_empty_tool_call_id_is_not_forwarded(Scenario::Convert(ConvertCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"","name":"n","content":"x"}"#,
        last: None,
        forward: true,
        urls: &[],
    },
    expected: r#"{"function_response":{"name":"n","response":{"content":"x"}}}"#,
}))]
#[case::conversion_leaves_the_message_untouched(Scenario::Preserved(PreservedCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_a__thought__sig","name":"n","content":[{"type":"text","text":"x"},{"type":"image","source":{"type":"base64","data":"QUJD"}}]}"#,
        last: Some(r#"{"tool_calls":[{"id":"call_a__thought__sig","function":{"name":"n","arguments":"{}"}}]}"#),
        forward: true,
        urls: &[],
    },
    expected: r#"[{"function_response":{"name":"n","response":{"content":"x"},"id":"call_a","parts":[{"inline_data":{"data":"QUJD","mime_type":"image/jpeg"}}]}}]"#,
}))]
#[case::a_user_role_is_refused(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"user","name":"n","content":"x"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Role,
}))]
#[case::a_missing_name_is_refused(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","content":"x"}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::MissingName,
}))]
#[case::an_unmatched_id_without_a_name_is_refused(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_c","content":"x"}"#,
        last: Some(r#"{"tool_calls":[{"id":"call_a","function":{"name":"first","arguments":"{}"}}]}"#),
        forward: false,
        urls: &[],
    },
    failure: Failure::MissingName,
}))]
#[case::a_matched_function_without_a_name_clears_the_message_name(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"call_b","name":"given","content":"x"}"#,
        last: Some(r#"{"tool_calls":[{"id":"call_b","function":{"arguments":"{}"}}]}"#),
        forward: false,
        urls: &[],
    },
    failure: Failure::MissingName,
}))]
#[case::a_text_part_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"text","text":5}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].text"),
}))]
#[case::image_source_data_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image","source":{"type":"base64","data":5}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].source.data"),
}))]
#[case::image_source_media_type_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image","source":{"type":"base64","data":"QUJD","media_type":5}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].source.media_type"),
}))]
#[case::image_url_must_be_a_string_or_an_object(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"input_image","image_url":5}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].image_url"),
}))]
#[case::image_url_url_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"image_url","image_url":{"url":5}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].image_url.url"),
}))]
#[case::file_data_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file_data":5}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].file_data"),
}))]
#[case::file_must_be_a_string_or_an_object(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file":5}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].file"),
}))]
#[case::nested_file_data_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":"c","name":"n","content":[{"type":"file","file":{"file_data":5}}]}"#,
        last: None,
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("content[0].file.file_data"),
}))]
#[case::a_matching_tool_call_id_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":5,"name":"n","content":"x"}"#,
        last: Some(r#"{"tool_calls":[]}"#),
        forward: false,
        urls: &[],
    },
    failure: Failure::Field("tool_call_id"),
}))]
#[case::a_forwarded_tool_call_id_must_hold_a_string(Scenario::Failure(FailureCase {
    call: Call {
        message: r#"{"role":"tool","tool_call_id":5,"name":"n","content":"x"}"#,
        last: None,
        forward: true,
        urls: &[],
    },
    failure: Failure::Field("tool_call_id"),
}))]
#[case::invalid_message_bytes_are_refused(Scenario::ParseFailure("{not json"))]
#[case::trailing_bytes_are_refused(Scenario::ParseFailure(r#"{"role":"tool"} trailing"#))]
#[case::oversized_message_bytes_are_refused(Scenario::OversizedMessage)]
#[case::a_message_at_the_byte_limit_parses(Scenario::MessageAtByteLimit)]
#[case::too_many_content_parts_are_refused(Scenario::TooManyParts)]
#[case::content_parts_at_the_limit_are_accepted(Scenario::MaxParts)]
#[case::too_many_previous_tool_calls_are_refused(Scenario::TooManyToolCalls)]
#[case::previous_tool_calls_at_the_limit_are_accepted(Scenario::MaxToolCalls)]
#[tokio::test]
async fn gemini_tool_results_behavior_tests_behavior(#[case] scenario: Scenario) {
    run(scenario).await;
}

#[rstest]
fn replays_the_gemini_tool_results_corpus(
    #[files("fuzz/corpus/gemini_tool_results/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(message) = parse_tool_result_message(&bytes) else {
        return;
    };
    let data_urls = BTreeMap::new();
    let source = MapSource {
        data_urls: &data_urls,
    };
    let runtime = Builder::new_current_thread().build().unwrap();
    drop(runtime.block_on(convert_tool_result_to_gemini(&message, None, true, &source)));
    drop(runtime.block_on(convert_tool_result_to_gemini(
        &message, None, false, &source,
    )));
}

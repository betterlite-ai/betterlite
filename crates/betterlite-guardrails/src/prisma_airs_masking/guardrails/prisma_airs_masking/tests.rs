#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{read, read_dir};
use std::path::Path;
use std::str::from_utf8;

use serde_json::value::RawValue;

use crate::prisma_airs_extraction::{Message as ExtractionMessage, Response as ExtractionResponse};
use crate::prisma_airs_masking::mcp::{
    McpRequest, McpServer, get_mcp_server_name, mask_mcp_arguments, parse_mcp_request,
};
use crate::prisma_airs_masking::{
    MaskContent, MaskedData, MaskedFunctionCall, MaskedResponseMessage, MaskedToolCall,
    MaskingError, RequestMessage, Response, ResponseChoice, ScanResult, get_masked_text,
    mask_messages, mask_response,
};

#[test]
fn selects_prompt_or_response_masked_data_for_the_requested_scan_direction() {
    let scan = ScanResult {
        prompt_masked_data: Some(MaskedData {
            data: Some("prompt".to_owned()),
        }),
        response_masked_data: Some(MaskedData {
            data: Some("response".to_owned()),
        }),
    };
    assert_eq!(get_masked_text(&scan, false), Some("prompt"));
    assert_eq!(get_masked_text(&scan, true), Some("response"));
}

#[test]
fn masks_last_user_message_and_preserves_non_text_content_parts() {
    let mut messages = [
        RequestMessage {
            role: Some("user".to_owned()),
            content: Some(MaskContent::Text("older".to_owned())),
        },
        RequestMessage {
            role: Some("user".to_owned()),
            content: Some(MaskContent::Parts(vec![
                raw(r#"{"type":"text","text":"secret"}"#),
                raw(r#"{"type":"image","url":"image"}"#),
            ])),
        },
    ];
    assert!(mask_messages(&mut messages, "masked").expect("mask"));
    assert!(matches!(&messages[0].content, Some(MaskContent::Text(text)) if text == "older"));
    let Some(MaskContent::Parts(parts)) = messages[1].content.as_ref() else {
        panic!("content remains a part list");
    };
    assert_eq!(parts[0].get(), r#"{"type":"text","text":"masked"}"#);
    assert_eq!(parts[1].get(), r#"{"type":"image","url":"image"}"#);
}

#[test]
fn masks_every_response_choice_and_tool_argument() {
    let mut response = response_fixture();
    mask_response(&mut response, "masked").expect("mask");
    assert_masked_response(&response);
}

fn response_fixture() -> Response {
    Response {
        choices: Some(vec![
            ResponseChoice {
                message: Some(MaskedResponseMessage {
                    content: Some(MaskContent::Text("secret".to_owned())),
                    tool_calls: Some(vec![
                        MaskedToolCall {
                            function: Some(MaskedFunctionCall {
                                arguments: Some("secret-1".to_owned()),
                            }),
                        },
                        MaskedToolCall {
                            function: Some(MaskedFunctionCall {
                                arguments: Some("secret-2".to_owned()),
                            }),
                        },
                    ]),
                    function_call: None,
                }),
            },
            ResponseChoice {
                message: Some(MaskedResponseMessage {
                    content: Some(MaskContent::Parts(vec![
                        raw(r#"{"type":"text","text":"secret-a"}"#),
                        raw(r#"{"type":"image"}"#),
                        raw(r#"{"type":"text","text":"secret-b"}"#),
                    ])),
                    tool_calls: None,
                    function_call: Some(MaskedFunctionCall {
                        arguments: Some("secret".to_owned()),
                    }),
                }),
            },
        ]),
    }
}

fn assert_masked_response(response: &Response) {
    let choices = response.choices.as_ref().expect("choices");
    let first = choices[0].message.as_ref().expect("first message");
    assert!(matches!(&first.content, Some(MaskContent::Text(text)) if text == "masked"));
    let first_tool_calls = first.tool_calls.as_ref().expect("tool calls");
    assert_eq!(first_tool_calls.len(), 2);
    for call in first_tool_calls {
        assert_eq!(
            call.function
                .as_ref()
                .expect("function")
                .arguments
                .as_deref(),
            Some("masked")
        );
    }
    let second = choices[1].message.as_ref().expect("second message");
    assert_eq!(
        second
            .function_call
            .as_ref()
            .expect("function call")
            .arguments
            .as_deref(),
        Some("masked")
    );
    let Some(MaskContent::Parts(parts)) = second.content.as_ref() else {
        panic!("content remains a part list");
    };
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].get(), r#"{"type":"text","text":"masked"}"#);
    assert_eq!(parts[1].get(), r#"{"type":"image"}"#);
    assert_eq!(parts[2].get(), r#"{"type":"text","text":"masked"}"#);
}

#[test]
fn mcp_masking_preserves_structured_type_and_updates_both_fields() {
    let mut request = mcp_request_with(&["arguments", "mcp_arguments"]);
    let original = raw(r#"{"secret":"value"}"#);
    mask_mcp_arguments(&mut request, &original, r#"{"secret":"masked"}"#).expect("valid JSON");
    assert_eq!(
        request.arguments.as_ref().expect("arguments").get(),
        r#"{"secret":"masked"}"#
    );
    assert_eq!(
        request
            .mcp_arguments
            .as_ref()
            .expect("mirrored arguments")
            .get(),
        r#"{"secret":"masked"}"#
    );
}

#[test]
fn rejects_non_json_mask_for_structured_mcp_arguments() {
    let mut request = mcp_request_with(&["arguments"]);
    let original = raw(r#"{"secret":"value"}"#);
    assert!(matches!(
        mask_mcp_arguments(&mut request, &original, "masked"),
        Err(MaskingError::InvalidMcpJson(_))
    ));
}

fn mcp_request_with(fields: &[&str]) -> McpRequest {
    McpRequest {
        mcp_server_name: None,
        server_name: None,
        server_id: None,
        arguments: fields
            .contains(&"arguments")
            .then(|| RawValue::NULL.to_owned()),
        mcp_arguments: fields
            .contains(&"mcp_arguments")
            .then(|| RawValue::NULL.to_owned()),
    }
}

#[test]
fn resolves_mcp_server_name_from_request_registry_or_unknown() {
    let (servers, names) = server_registry();
    let request_name = request(Some("request-name"), None);
    assert_eq!(
        get_mcp_server_name(&request_name, &servers, &names, "tool"),
        "request-name"
    );
    let request_id = request(None, Some("server-id"));
    assert_eq!(
        get_mcp_server_name(&request_id, &servers, &names, "missing"),
        "registry-name"
    );
    let missing = request(None, None);
    assert_eq!(
        get_mcp_server_name(&missing, &BTreeMap::new(), &names, "missing"),
        "unknown"
    );
}

fn server_registry() -> (BTreeMap<String, McpServer>, BTreeMap<String, String>) {
    let servers = BTreeMap::from([(
        "server-id".to_owned(),
        McpServer {
            alias: Some("registry-name".to_owned()),
            server_name: None,
            name: None,
            server_id: None,
        },
    )]);
    let names = BTreeMap::from([("tool".to_owned(), "tool-server".to_owned())]);
    (servers, names)
}

fn request(mcp_server_name: Option<&str>, server_id: Option<&str>) -> McpRequest {
    McpRequest {
        mcp_server_name: mcp_server_name.map(ToOwned::to_owned),
        server_name: None,
        server_id: server_id.map(ToOwned::to_owned),
        arguments: None,
        mcp_arguments: None,
    }
}

fn raw(json: &str) -> Box<RawValue> {
    RawValue::from_string(json.to_owned()).expect("valid raw JSON")
}

#[rstest::rstest]
#[case::object(r#"{"arguments":{}}"#, "{}", r#"{"masked":true}"#)]
#[case::array(r#"{"mcp_arguments":[]}"#, "[]", "[1]")]
#[case::parsed_scalar(r#"{"arguments":{}}"#, "{}", "42")]
#[case::null_field(r#"{"arguments":null}"#, "null", "masked")]
#[case::string(r#"{"arguments":"secret"}"#, r#""secret""#, "masked")]
fn rewrites_each_mcp_argument_shape(
    #[case] input: &str,
    #[case] original: &str,
    #[case] masked: &str,
) {
    let mut request = parse_mcp_request(input.as_bytes()).expect("request");
    mask_mcp_arguments(&mut request, &raw(original), masked).expect("mask");
    let updated = request
        .arguments
        .as_ref()
        .or(request.mcp_arguments.as_ref())
        .expect("field");
    let expected = if original == "{}" || original == "[]" {
        masked.to_owned()
    } else {
        serde_json::to_string(masked).expect("string")
    };
    assert_eq!(updated.get(), expected);
}

#[test]
fn parse_mcp_request_rejects_duplicate_known_fields() {
    let error = parse_mcp_request(br#"{"server_name":"a","server_name":"b"}"#)
        .expect_err("duplicate field");
    assert!(error.to_string().contains("server_name"), "{error}");
}

#[test]
fn parse_mcp_request_distinguishes_missing_from_null_arguments() {
    let absent = parse_mcp_request(b"{}").expect("request");
    assert!(absent.arguments.is_none());
    assert!(absent.mcp_arguments.is_none());

    let present_null = parse_mcp_request(br#"{"arguments":null}"#).expect("request");
    assert_eq!(
        present_null.arguments.as_ref().expect("present").get(),
        "null"
    );
    assert!(present_null.mcp_arguments.is_none());
}

#[test]
fn missing_mcp_fields_are_rejected_without_inventing_arguments() {
    let mut request = parse_mcp_request(b"{}").expect("request");
    assert!(matches!(
        mask_mcp_arguments(&mut request, &raw("{}"), "{}"),
        Err(MaskingError::MissingMcpArguments)
    ));
}

#[rstest::rstest]
#[case::explicit(r#"{"mcp_server_name":"first","server_name":"second"}"#, "{}", "first")]
#[case::secondary(r#"{"server_name":"second"}"#, "{}", "second")]
#[case::empty_explicit(r#"{"mcp_server_name":"","server_name":"second"}"#, "{}", "second")]
#[case::alias(
    r#"{"server_id":"id"}"#,
    r#"{"alias":"alias","server_name":"server"}"#,
    "alias"
)]
#[case::server(
    r#"{"server_id":"id"}"#,
    r#"{"server_name":"server","name":"name"}"#,
    "server"
)]
#[case::name(
    r#"{"server_id":"id"}"#,
    r#"{"name":"name","server_id":"fallback"}"#,
    "name"
)]
#[case::id(r#"{"server_id":"id"}"#, r#"{"server_id":"fallback"}"#, "fallback")]
#[case::unnamed(r#"{"server_id":"id"}"#, "{}", "unknown")]
#[case::tool(r#"{"server_id":"missing"}"#, "{}", "tool-server")]
fn server_names_follow_precedence(
    #[case] input: &str,
    #[case] server: &str,
    #[case] expected: &str,
) {
    let request = parse_mcp_request(input.as_bytes()).expect("request");
    let servers = BTreeMap::from([(
        "id".to_owned(),
        serde_json::from_str(server).expect("server"),
    )]);
    let names = BTreeMap::from([("tool".to_owned(), "tool-server".to_owned())]);
    assert_eq!(
        get_mcp_server_name(&request, &servers, &names, "tool"),
        expected
    );
}

#[rstest::rstest]
#[case::developer_only(r#"[{"role":"developer","content":"secret"}]"#, 0)]
#[case::skips_a_role_eligible_message_with_no_content(
    r#"[{"role":"user","content":"older"},{"role":"user"}]"#,
    0
)]
fn masks_the_last_eligible_role_message_with_content(#[case] input: &str, #[case] index: usize) {
    let mut messages: Vec<RequestMessage> = serde_json::from_str(input).expect("messages");
    assert!(mask_messages(&mut messages, "masked").expect("mask"));
    assert!(matches!(&messages[index].content, Some(MaskContent::Text(text)) if text == "masked"));
}

#[test]
fn mask_messages_fails_closed_when_no_message_is_eligible() {
    let mut messages: Vec<RequestMessage> = serde_json::from_str("[]").expect("messages");
    assert!(matches!(
        mask_messages(&mut messages, "masked"),
        Err(MaskingError::NoMaskableMessage)
    ));
}

#[test]
fn masks_a_trailing_developer_message_holding_a_credential_not_an_earlier_user_message() {
    let mut messages: Vec<RequestMessage> = serde_json::from_str(
        r#"[{"role":"user","content":"hello"},{"role":"developer","content":"api_key=sk-secret"}]"#,
    )
    .expect("messages");
    assert!(mask_messages(&mut messages, "masked").expect("mask"));
    assert!(matches!(&messages[0].content, Some(MaskContent::Text(text)) if text == "hello"));
    assert!(matches!(&messages[1].content, Some(MaskContent::Text(text)) if text == "masked"));
}

#[rstest::rstest]
#[case::absent("{}", None)]
#[case::missing_data(r#"{"prompt_masked_data":{}}"#, None)]
#[case::empty_data(r#"{"prompt_masked_data":{"data":""}}"#, Some(""))]
fn absent_masked_data_has_no_replacement(#[case] input: &str, #[case] expected: Option<&str>) {
    let scan: ScanResult = serde_json::from_str(input).expect("scan");
    assert_eq!(get_masked_text(&scan, false), expected);
    assert_eq!(get_masked_text(&scan, true), None);
}

#[test]
fn replays_prisma_content_corpus() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/prisma_content");
    let entries: Vec<_> = read_dir(folder).expect("corpus").collect();
    assert!(!entries.is_empty());
    assert!(entries.into_iter().all(|entry| {
        let bytes = read(entry.expect("entry").path()).expect("fixture");
        let messages = serde_json::from_slice::<Vec<ExtractionMessage>>(&bytes);
        let response = serde_json::from_slice::<ExtractionResponse>(&bytes);
        let scan = serde_json::from_slice::<ScanResult>(&bytes);
        let response_mask = serde_json::from_slice::<Response>(&bytes);
        let request = parse_mcp_request(&bytes);
        if let Ok(mut parsed) = request {
            let text = from_utf8(&bytes).expect("JSON");
            let _result = mask_mcp_arguments(&mut parsed, RawValue::NULL, text);
        }
        messages.is_ok() || response.is_ok() || scan.is_ok() || response_mask.is_ok()
    }));
}

#[test]
fn mask_messages_rejects_a_replacement_budget_that_would_allocate_gigabytes() {
    const SPEC_MAX_MASKED_OUTPUT_BYTES: usize = 0x0010_0000;
    let masked_text = "x".repeat(SPEC_MAX_MASKED_OUTPUT_BYTES);
    let mut messages = [RequestMessage {
        role: Some("user".to_owned()),
        content: Some(MaskContent::Parts(vec![
            raw(r#"{"type":"text","text":"a"}"#),
            raw(r#"{"type":"text","text":"b"}"#),
        ])),
    }];
    let error = mask_messages(&mut messages, &masked_text).expect_err("budget exceeded");
    assert!(matches!(
        error,
        MaskingError::ReplacementTooLarge { limit } if limit == SPEC_MAX_MASKED_OUTPUT_BYTES
    ));
    let Some(MaskContent::Parts(parts)) = messages[0].content.as_ref() else {
        panic!("content remains a part list");
    };
    assert_eq!(parts[0].get(), r#"{"type":"text","text":"a"}"#);
}

#[test]
fn mask_messages_allows_a_replacement_budget_exactly_at_the_cap() {
    const SPEC_MAX_MASKED_OUTPUT_BYTES: usize = 0x0010_0000;
    let masked_text = "x".repeat(SPEC_MAX_MASKED_OUTPUT_BYTES);
    let mut messages = [RequestMessage {
        role: Some("user".to_owned()),
        content: Some(MaskContent::Text("secret".to_owned())),
    }];
    assert!(mask_messages(&mut messages, &masked_text).expect("mask"));
    assert!(matches!(&messages[0].content, Some(MaskContent::Text(text)) if *text == masked_text));
}

#[test]
fn mask_response_rejects_a_replacement_budget_spread_across_choices_and_tool_arguments() {
    const SPEC_MAX_MASKED_OUTPUT_BYTES: usize = 0x0010_0000;
    let masked_text = "x".repeat(200_000);
    let mut response = response_fixture();
    let error = mask_response(&mut response, &masked_text).expect_err("budget exceeded");
    assert!(matches!(
        error,
        MaskingError::ReplacementTooLarge { limit } if limit == SPEC_MAX_MASKED_OUTPUT_BYTES
    ));
    let choices = response.choices.as_ref().expect("choices");
    assert!(matches!(
        &choices[0].message.as_ref().expect("first message").content,
        Some(MaskContent::Text(text)) if text == "secret"
    ));
}

#[test]
fn mask_messages_does_not_charge_the_budget_for_a_preserved_non_text_part() {
    const MASKED_TEXT_BYTES: usize = 614_400;
    let masked_text = "x".repeat(MASKED_TEXT_BYTES);
    let mut messages = [RequestMessage {
        role: Some("user".to_owned()),
        content: Some(MaskContent::Parts(vec![
            raw(r#"{"type":"text","text":"secret"}"#),
            raw(r#"{"type":"image","url":"image"}"#),
        ])),
    }];
    assert!(mask_messages(&mut messages, &masked_text).expect("mask"));
    let Some(MaskContent::Parts(parts)) = messages[0].content.as_ref() else {
        panic!("content remains a part list");
    };
    assert_eq!(
        parts[0].get(),
        format!(r#"{{"type":"text","text":"{masked_text}"}}"#)
    );
    assert_eq!(parts[1].get(), r#"{"type":"image","url":"image"}"#);
}

#[test]
fn mask_response_does_not_charge_the_budget_for_a_preserved_non_text_part() {
    const MASKED_TEXT_BYTES: usize = 614_400;
    let masked_text = "x".repeat(MASKED_TEXT_BYTES);
    let mut response = Response {
        choices: Some(vec![ResponseChoice {
            message: Some(MaskedResponseMessage {
                content: Some(MaskContent::Parts(vec![
                    raw(r#"{"type":"text","text":"secret"}"#),
                    raw(r#"{"type":"image"}"#),
                ])),
                tool_calls: None,
                function_call: None,
            }),
        }]),
    };
    mask_response(&mut response, &masked_text).expect("mask");
    let message = response.choices.as_ref().expect("choices")[0]
        .message
        .as_ref()
        .expect("message");
    let Some(MaskContent::Parts(parts)) = message.content.as_ref() else {
        panic!("content remains a part list");
    };
    assert_eq!(
        parts[0].get(),
        format!(r#"{{"type":"text","text":"{masked_text}"}}"#)
    );
    assert_eq!(parts[1].get(), r#"{"type":"image"}"#);
}

#[test]
fn deserializes_multimodal_content_before_masking() {
    let mut messages: Vec<RequestMessage> = serde_json::from_str(r#"[{"role":"user","content":[{"type":"text","text":"secret"},{"type":"image_url","image_url":{"url":"image"}}]}]"#).expect("wire messages");
    assert!(mask_messages(&mut messages, "safe").expect("mask"));
    let Some(MaskContent::Parts(parts)) = &messages[0].content else {
        panic!("parts")
    };
    assert_eq!(parts[0].get(), r#"{"type":"text","text":"safe"}"#);
    assert_eq!(
        parts[1].get(),
        r#"{"type":"image_url","image_url":{"url":"image"}}"#
    );
}

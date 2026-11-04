#![cfg(test)]

use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::prisma_airs_masking::{
    MaskContent, MaskedData, MaskedFunctionCall, MaskedResponseMessage, MaskedToolCall,
    MaskingError, McpRequest, McpServer, RequestMessage, Response, ResponseChoice, ScanResult,
    get_masked_text, get_mcp_server_name, mask_mcp_arguments, mask_messages, mask_response,
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
    mask_messages(&mut messages, "masked").expect("mask");
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
                    tool_calls: Some(vec![MaskedToolCall {
                        function: Some(MaskedFunctionCall {
                            arguments: Some("secret".to_owned()),
                        }),
                    }]),
                    function_call: None,
                }),
            },
            ResponseChoice {
                message: Some(MaskedResponseMessage {
                    content: Some(MaskContent::Parts(vec![
                        raw(r#"{"type":"text","text":"secret"}"#),
                        raw(r#"{"type":"image"}"#),
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
    assert_eq!(
        first.tool_calls.as_ref().expect("tool calls")[0]
            .function
            .as_ref()
            .expect("function")
            .arguments
            .as_deref(),
        Some("masked")
    );
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
    assert_eq!(parts[1].get(), r#"{"type":"image"}"#);
}

#[test]
fn mcp_masking_preserves_structured_type_and_updates_both_fields() {
    let mut request = McpRequest {
        mcp_server_name: None,
        server_name: None,
        server_id: None,
        arguments: Some(raw(r#"{"secret":"value"}"#)),
        mcp_arguments: Some(raw(r#"{"secret":"value"}"#)),
    };
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
    let mut request = McpRequest {
        mcp_server_name: None,
        server_name: None,
        server_id: None,
        arguments: Some(raw(r#"{"secret":"value"}"#)),
        mcp_arguments: None,
    };
    let original = raw(r#"{"secret":"value"}"#);
    assert!(matches!(
        mask_mcp_arguments(&mut request, &original, "masked"),
        Err(MaskingError::InvalidMcpJson(_))
    ));
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

#![cfg(test)]

use serde_json::{Map, Value, json};

use crate::prisma_airs_masking::{
    MaskingError, get_masked_text, get_mcp_server_name, mask_mcp_arguments, mask_messages,
    mask_response,
};

#[test]
fn selects_prompt_or_response_masked_data_for_the_requested_scan_direction() {
    let scan = json!({
        "prompt_masked_data":{"data":"prompt"},
        "response_masked_data":{"data":"response"}
    });
    assert_eq!(get_masked_text(&scan, false), Some("prompt"));
    assert_eq!(get_masked_text(&scan, true), Some("response"));
    assert_eq!(
        get_masked_text(&json!({"prompt_masked_data":null}), false),
        None
    );
}

#[test]
fn masks_last_user_message_and_preserves_non_text_content_parts() {
    let mut messages = json!([
        {"role":"user", "content":"older"},
        {"role":"user", "content":[{"type":"text", "text":"secret"}, {"type":"image", "url":"image"}]}
    ]);
    mask_messages(messages.as_array_mut().expect("messages"), "masked");
    assert_eq!(messages[0]["content"], "older");
    assert_eq!(messages[1]["content"][0]["text"], "masked");
    assert_eq!(
        messages[1]["content"][1],
        json!({"type":"image", "url":"image"})
    );
}

#[test]
fn masks_every_response_choice_and_tool_argument() {
    let mut response = json!({"choices":[
        {"message":{"content":"secret", "tool_calls":[{"function":{"arguments":"secret"}}]}},
        {"message":{"content":[{"type":"text", "text":"secret"}, {"type":"image"}], "function_call":{"arguments":"secret"}}}
    ]});
    mask_response(&mut response, "masked");
    assert_eq!(response["choices"][0]["message"]["content"], "masked");
    assert_eq!(
        response["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        "masked"
    );
    assert_eq!(
        response["choices"][1]["message"]["function_call"]["arguments"],
        "masked"
    );
    assert_eq!(
        response["choices"][1]["message"]["content"][1],
        json!({"type":"image"})
    );
}

#[test]
fn mcp_masking_preserves_structured_type_and_updates_both_fields() {
    let original = json!({"secret":"value"});
    let mut request = Map::from_iter([
        ("arguments".to_owned(), original.clone()),
        ("mcp_arguments".to_owned(), original.clone()),
    ]);
    mask_mcp_arguments(&mut request, &original, "{\"secret\":\"masked\"}").expect("valid JSON");
    assert_eq!(request.get("arguments"), Some(&json!({"secret":"masked"})));
    assert_eq!(
        request.get("mcp_arguments"),
        Some(&json!({"secret":"masked"}))
    );
}

#[test]
fn rejects_non_json_mask_for_structured_mcp_arguments() {
    let mut request = Map::from_iter([("arguments".to_owned(), json!({"secret":"value"}))]);
    assert_eq!(
        mask_mcp_arguments(&mut request, &json!({}), "masked"),
        Err(MaskingError::InvalidMcpJson)
    );
}

#[test]
fn resolves_mcp_server_name_from_request_registry_or_unknown() {
    let registry = Map::from_iter([
        ("server-id".to_owned(), json!({"alias":"registry-name"})),
        ("tool".to_owned(), json!("tool-server")),
    ]);
    assert_eq!(
        get_mcp_server_name(
            &json!({"mcp_server_name":"request-name"}),
            &registry,
            "tool"
        ),
        "request-name"
    );
    assert_eq!(
        get_mcp_server_name(&json!({"server_id":"server-id"}), &registry, "missing"),
        "registry-name"
    );
    assert_eq!(
        get_mcp_server_name(&Value::Null, &Map::new(), "missing"),
        "unknown"
    );
}

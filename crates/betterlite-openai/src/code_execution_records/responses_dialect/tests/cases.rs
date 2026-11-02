#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
#[test]
fn normalize_messages_wraps_a_string_as_a_single_user_message() {
    let normalized = normalize_messages(&json!("hello"));
    assert_eq!(
        normalized,
        vec![test_object([
            ("role", json!("user")),
            ("content", json!("hello"))
        ])]
    );
}

#[cfg(test)]
#[test]
fn normalize_messages_copies_a_list_as_is() {
    let messages = Value::Array(vec![test_object([
        ("role", json!("user")),
        ("content", json!("hi")),
    ])]);
    assert_eq!(
        normalize_messages(&messages),
        messages.as_array().unwrap().clone()
    );
}

#[cfg(test)]
#[test]
fn normalize_messages_returns_empty_for_other_types() {
    assert_eq!(normalize_messages(&json!(42_i64)), Vec::<Value>::new());
    assert_eq!(normalize_messages(&json!(null)), Vec::<Value>::new());
}

#[cfg(test)]
#[test]
fn is_code_execution_call_matches_function_call_items_named_litellm_code_execution() {
    let item = test_object([
        ("type", json!("function_call")),
        ("name", json!(CODE_EXECUTION_TOOL_NAME)),
    ]);
    assert!(is_code_execution_call(&item));
}

#[cfg(test)]
#[test]
fn is_code_execution_call_rejects_other_types_and_names() {
    assert!(!is_code_execution_call(&test_object([
        ("type", json!("function_call")),
        ("name", json!("web_search"))
    ])));
    assert!(!is_code_execution_call(&test_object([
        ("type", json!("message")),
        ("name", json!(CODE_EXECUTION_TOOL_NAME)),
    ])));
    assert!(!is_code_execution_call(&json!("not an object")));
}

#[cfg(test)]
#[test]
fn extract_tool_calls_finds_code_execution_calls_in_the_response_output() {
    let response = test_object([(
        "output",
        Value::Array(vec![
            test_object([
                ("type", json!("function_call")),
                ("name", json!(CODE_EXECUTION_TOOL_NAME)),
                ("call_id", json!("call_1")),
                ("arguments", json!("{\"code\": \"1\"}")),
            ]),
            test_object([
                ("type", json!("function_call")),
                ("name", json!("web_search")),
                ("call_id", json!("call_2")),
                ("arguments", json!("{}")),
            ]),
            test_object([("type", json!("message")), ("content", json!("done"))]),
        ]),
    )]);
    let extracted = extract_tool_calls(&response);
    assert_eq!(extracted.len(), 1);
    assert_eq!(extracted[0].call_id, Some("call_1".to_owned()));
    assert_eq!(extracted[0].id, None);
    assert_eq!(extracted[0].arguments, "{\"code\": \"1\"}");
}

#[cfg(test)]
#[test]
fn extract_tool_calls_returns_empty_when_output_is_not_a_list() {
    assert_eq!(extract_tool_calls(&json!({})), Vec::new());
}

#[cfg(test)]
#[test]
fn merge_calls_inserts_before_the_first_message_item() {
    let output = Value::Array(vec![
        test_object([("type", json!("reasoning"))]),
        test_object([("type", json!("message")), ("content", json!("done"))]),
    ]);
    let calls = vec![test_object([("type", json!("function_call_output"))])];
    let merged = merge_calls(&output, &calls).unwrap();
    assert_eq!(
        merged,
        vec![
            test_object([("type", json!("reasoning"))]),
            test_object([("type", json!("function_call_output"))]),
            test_object([("type", json!("message")), ("content", json!("done"))]),
        ]
    );
}

#[cfg(test)]
#[test]
fn merge_calls_appends_at_the_end_when_no_message_item_is_present() {
    let output = Value::Array(vec![test_object([("type", json!("reasoning"))])]);
    let calls = vec![test_object([("type", json!("function_call_output"))])];
    let merged = merge_calls(&output, &calls).unwrap();
    assert_eq!(
        merged,
        vec![
            test_object([("type", json!("reasoning"))]),
            test_object([("type", json!("function_call_output"))])
        ]
    );
}

#[cfg(test)]
#[test]
fn merge_calls_returns_none_when_output_is_not_a_list() {
    assert_eq!(merge_calls(&json!("not a list"), &[]), None);
}

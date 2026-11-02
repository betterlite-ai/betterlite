#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
#[test]
fn agentic_tool_calls_narrows_every_dict_item_in_the_list() {
    let tools = test_object([(
        "tool_calls",
        Value::Array(vec![
            test_object([
                ("id", json!("call_1")),
                ("call_id", json!("resp_1")),
                ("arguments", json!("{}")),
            ]),
            json!("not a dict"),
            test_object([
                ("call_id", json!("resp_2")),
                ("arguments", json!("{\"code\": \"2\"}")),
            ]),
        ]),
    )]);
    let narrowed = agentic_tool_calls(tools.as_object().unwrap());
    assert_eq!(narrowed.len(), 2);
    assert_eq!(narrowed[0].id, Some("call_1".to_owned()));
    assert_eq!(narrowed[1].call_id, Some("resp_2".to_owned()));
    assert_eq!(narrowed[1].arguments, "{\"code\": \"2\"}");
}

#[cfg(test)]
#[test]
fn agentic_tool_calls_returns_empty_when_tool_calls_is_missing_or_not_a_list() {
    assert_eq!(
        agentic_tool_calls(json!({}).as_object().unwrap()),
        Vec::new()
    );
    assert_eq!(
        agentic_tool_calls(
            test_object([("tool_calls", json!("not a list"))])
                .as_object()
                .unwrap()
        ),
        Vec::new()
    );
}

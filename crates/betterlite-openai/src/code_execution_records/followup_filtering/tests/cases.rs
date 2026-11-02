#[cfg(test)]
use rstest::rstest;
#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::CODE_EXECUTION_TOOL_NAME;
#[cfg(test)]
use super::test_object;

#[cfg(test)]
#[rstest]
#[case::litellm_logging_obj("litellm_logging_obj")]
#[case::acompletion("acompletion")]
#[case::websearch_marker("_websearch_interception_active")]
#[case::headroom_marker("_headroom_interception_converted_stream")]
fn agentic_loop_kwargs_removes_logging_globals_and_other_interceptors_markers(
    #[case] dropped_key: &str,
) {
    let mut kwargs = Map::new();
    kwargs.insert(dropped_key.to_owned(), json!(true));
    kwargs.insert("model".to_owned(), json!("gpt-5"));
    let filtered = agentic_loop_kwargs(&kwargs);
    assert_eq!(filtered.keys().collect::<Vec<_>>(), ["model"]);
}

#[cfg(test)]
#[test]
fn agentic_loop_kwargs_keeps_this_interceptors_own_markers() {
    let mut kwargs = Map::new();
    kwargs.insert(
        "_code_interpreter_interception_active".to_owned(),
        json!(true),
    );
    let filtered = agentic_loop_kwargs(&kwargs);
    assert_eq!(
        filtered.get("_code_interpreter_interception_active"),
        Some(&json!(true))
    );
}

#[cfg(test)]
#[test]
fn tools_replaces_code_interpreter_entries_and_keeps_the_rest() {
    let requested = Value::Array(vec![
        test_object([("type", json!("code_interpreter"))]),
        test_object([("type", json!("web_search"))]),
    ]);
    let replaced = tools(&requested, CallDialect::Responses).unwrap();
    assert_eq!(replaced[0]["name"], json!(CODE_EXECUTION_TOOL_NAME));
    assert_eq!(replaced[1], test_object([("type", json!("web_search"))]));
}

#[cfg(test)]
#[test]
fn tools_returns_none_when_the_value_is_not_a_list() {
    assert_eq!(tools(&json!("not a list"), CallDialect::Responses), None);
}

#[cfg(test)]
#[test]
fn optional_params_drops_tools_and_a_code_interpreter_tool_choice() {
    let mut params = Map::new();
    params.insert(
        "tools".to_owned(),
        Value::Array(vec![test_object([("type", json!("code_interpreter"))])]),
    );
    params.insert(
        "tool_choice".to_owned(),
        test_object([("type", json!("code_interpreter"))]),
    );
    params.insert("temperature".to_owned(), json!(1_i64));
    let filtered = optional_params(&params);
    assert_eq!(filtered.keys().collect::<Vec<_>>(), ["temperature"]);
}

#[cfg(test)]
#[test]
fn optional_params_keeps_a_tool_choice_that_does_not_target_code_interpreter() {
    let mut params = Map::new();
    params.insert("tool_choice".to_owned(), json!("auto"));
    let filtered = optional_params(&params);
    assert_eq!(filtered.get("tool_choice"), Some(&json!("auto")));
}

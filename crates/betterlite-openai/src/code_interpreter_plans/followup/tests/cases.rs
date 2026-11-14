#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use crate::code_execution_records::tool_shapes::function_tool;

#[cfg(test)]
use super::{followup_values, metadata_value, test_map};

#[cfg(test)]
#[test]
fn tools_rewrite_code_interpreter_tools_and_keep_the_others() {
    let weather = json!({"type": "function", "name": "get_weather"});
    let requested = json!([{"type": "code_interpreter"}, weather]);
    let rewritten = tools(Some(&requested), CallDialect::Responses)
        .unwrap()
        .unwrap();
    assert_eq!(
        followup_values(&rewritten),
        vec![
            serde_json::to_value(function_tool(CallDialect::Responses)).unwrap(),
            weather
        ]
    );
    let chat_rewritten = tools(Some(&requested), CallDialect::ChatCompletion)
        .unwrap()
        .unwrap();
    assert_eq!(
        followup_values(&chat_rewritten)[0],
        serde_json::to_value(function_tool(CallDialect::ChatCompletion)).unwrap()
    );
}

#[cfg(test)]
#[test]
fn tools_are_absent_without_a_usable_tools_list() {
    assert!(tools(None, CallDialect::Responses).unwrap().is_none());
    assert!(
        tools(Some(&json!("auto")), CallDialect::Responses)
            .unwrap()
            .is_none()
    );
    assert!(
        tools(Some(&json!([])), CallDialect::Responses)
            .unwrap()
            .unwrap()
            .is_empty()
    );
}

#[cfg(test)]
#[test]
fn params_drop_tools_and_only_a_code_interpreter_tool_choice() {
    let request_params = test_map([
        ("tools", json!([{"type": "code_interpreter"}])),
        ("tool_choice", json!({"type": "code_interpreter"})),
        ("temperature", json!(0.2_f64)),
    ]);
    let filtered = raw_map(&followup_filtering::optional_params(&request_params)).unwrap();
    assert!(filtered.get("tools").is_none());
    assert!(filtered.get("tool_choice").is_none());
    assert_eq!(metadata_value(&filtered, "temperature"), json!(0.2_f64));

    let automatic = test_map([
        ("tool_choice", json!("auto")),
        ("temperature", json!(0.1_f64)),
    ]);
    let kept = raw_map(&followup_filtering::optional_params(&automatic)).unwrap();
    assert_eq!(metadata_value(&kept, "tool_choice"), json!("auto"));
}

#[cfg(test)]
#[test]
fn kwargs_drop_the_logging_object_and_other_interceptors() {
    let request_kwargs = test_map([
        ("model", json!("openai/gpt-5")),
        ("litellm_logging_obj", json!("obj")),
        ("acompletion", json!(true)),
        ("_websearch_interception_active", json!(true)),
        ("_code_interpreter_interception_active", json!(true)),
        ("max_agentic_loops", json!(3_i32)),
    ]);
    let filtered = raw_map(&followup_filtering::agentic_loop_kwargs(&request_kwargs)).unwrap();
    assert_eq!(metadata_value(&filtered, "model"), json!("openai/gpt-5"));
    assert_eq!(
        metadata_value(&filtered, "_code_interpreter_interception_active"),
        json!(true)
    );
    assert_eq!(metadata_value(&filtered, "max_agentic_loops"), json!(3_i32));
    assert!(filtered.get("litellm_logging_obj").is_none());
    assert!(filtered.get("acompletion").is_none());
    assert!(filtered.get("_websearch_interception_active").is_none());
}

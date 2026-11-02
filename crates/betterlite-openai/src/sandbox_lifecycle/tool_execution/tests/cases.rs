#[cfg(test)]
use serde_json::json;

#[cfg(test)]
use super::{MockConfig, MockRegistry, MockToolRegistry, block_on, test_object};

#[cfg(test)]
const fn provider<'provider>(
    config: &'provider MockConfig,
    registry: &'provider MockRegistry,
    tool_registry: &'provider MockToolRegistry,
) -> SandboxProvider<'provider, MockConfig, MockRegistry, MockToolRegistry> {
    SandboxProvider {
        sandbox_config: Some(config),
        registry,
        tool_registry,
        sandbox_tool_name: None,
    }
}

#[cfg(test)]
#[test]
fn run_tool_call_executes_code_and_returns_stdout_when_there_is_no_error() {
    let config = MockConfig::succeeding(test_object([("stdout", json!("hello"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&config, &registry, &tool_registry);
    let container = test_object([("id", json!("container-1"))]);
    let output = block_on(run_tool_call(
        &sandbox,
        None,
        &container,
        r#"{"code": "print(1)"}"#,
    ))
    .unwrap();
    assert_eq!(output, "hello");
    let run_calls = config.calls.run.lock().unwrap();
    assert_eq!(run_calls.as_slice(), [(container, "print(1)".to_owned())]);
}

#[cfg(test)]
#[test]
fn run_tool_call_runs_empty_code_when_arguments_is_an_empty_string() {
    let config = MockConfig::succeeding(test_object([("stdout", json!("ran"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&config, &registry, &tool_registry);
    let container = test_object([("id", json!("container-1"))]);
    let output = block_on(run_tool_call(&sandbox, None, &container, "")).unwrap();
    assert_eq!(output, "ran");
}

#[cfg(test)]
#[test]
fn run_tool_call_returns_the_execution_error_message_when_the_result_has_an_error() {
    let config = MockConfig::succeeding(test_object([
        ("stdout", json!("")),
        ("error", test_object([("value", json!("boom"))])),
    ]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&config, &registry, &tool_registry);
    let container = test_object([("id", json!("container-1"))]);
    let output = block_on(run_tool_call(
        &sandbox,
        None,
        &container,
        r#"{"code": "1/0"}"#,
    ))
    .unwrap();
    assert_eq!(output, "[execution error] boom");
}

#[cfg(test)]
#[test]
fn run_tool_call_propagates_lifecycle_errors_from_running_the_code() {
    let config = MockConfig::failing("sandbox unreachable");
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&config, &registry, &tool_registry);
    let container = test_object([("id", json!("container-1"))]);
    let error = block_on(run_tool_call(
        &sandbox,
        None,
        &container,
        r#"{"code": "1"}"#,
    ))
    .unwrap_err();
    assert!(matches!(error, ToolExecutionError::Lifecycle(_)));
}

#[cfg(test)]
#[test]
fn invalid_arguments_returns_the_friendly_message_when_the_json_does_not_parse() {
    let config = MockConfig::failing("must not run");
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&config, &registry, &tool_registry);
    let container = test_object([("id", json!("container-1"))]);
    let output = block_on(run_tool_call(&sandbox, None, &container, "not json")).unwrap();
    assert_eq!(output, INVALID_ARGUMENTS_MESSAGE);
}

#[cfg(test)]
#[test]
fn invalid_arguments_rejects_valid_json_that_is_not_an_object() {
    let config = MockConfig::failing("must not run");
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&config, &registry, &tool_registry);
    let container = test_object([("id", json!("container-1"))]);
    let error = block_on(run_tool_call(&sandbox, None, &container, "5")).unwrap_err();
    assert!(matches!(error, ToolExecutionError::ArgumentsNotAnObject));
}

#[cfg(test)]
#[test]
fn invalid_arguments_treats_a_missing_code_field_as_empty_code() {
    let config = MockConfig::succeeding(test_object([("stdout", json!("ran"))]));
    let registry = MockRegistry::default();
    let tool_registry = MockToolRegistry::default();
    let sandbox = provider(&config, &registry, &tool_registry);
    let container = test_object([("id", json!("container-1"))]);
    let output = block_on(run_tool_call(&sandbox, None, &container, "{}")).unwrap();
    assert_eq!(output, "ran");
}

#[cfg(test)]
#[test]
fn error_message_extracts_the_value_field_when_present() {
    let result = test_object([(
        "error",
        test_object([
            ("value", json!("division by zero")),
            ("name", json!("ZeroDivisionError")),
        ]),
    )]);
    assert_eq!(
        error_message(&result),
        Some("[execution error] division by zero".to_owned())
    );
}

#[cfg(test)]
#[test]
fn error_message_falls_back_to_the_name_field_when_value_is_absent() {
    let result = test_object([("error", test_object([("name", json!("ZeroDivisionError"))]))]);
    assert_eq!(
        error_message(&result),
        Some("[execution error] ZeroDivisionError".to_owned())
    );
}

#[cfg(test)]
#[test]
fn error_message_falls_back_to_the_name_field_when_value_is_null() {
    let result = test_object([(
        "error",
        test_object([("value", json!(null)), ("name", json!("ZeroDivisionError"))]),
    )]);
    assert_eq!(
        error_message(&result),
        Some("[execution error] ZeroDivisionError".to_owned())
    );
}

#[cfg(test)]
#[test]
fn error_message_keeps_an_explicit_empty_string_value_instead_of_falling_back() {
    let result = test_object([(
        "error",
        test_object([("value", json!("")), ("name", json!("ZeroDivisionError"))]),
    )]);
    assert_eq!(
        error_message(&result),
        Some("[execution error] ".to_owned())
    );
}

#[cfg(test)]
#[test]
fn error_message_returns_none_when_there_is_no_error_field() {
    let result = test_object([("stdout", json!("ok"))]);
    assert_eq!(error_message(&result), None);
}

#[cfg(test)]
#[test]
fn error_message_returns_none_when_the_error_field_is_null() {
    let result = test_object([("error", json!(null)), ("stdout", json!("ok"))]);
    assert_eq!(error_message(&result), None);
}

#[cfg(test)]
#[test]
fn error_message_does_not_panic_when_the_error_field_is_not_an_object() {
    let result = test_object([("error", json!("boom"))]);
    assert_eq!(error_message(&result), None);
}

#[cfg(test)]
#[test]
fn error_message_falls_back_to_the_traceback_field_when_value_and_name_are_null() {
    let result = test_object([(
        "error",
        test_object([
            ("name", json!(null)),
            ("value", json!(null)),
            ("traceback", json!("Traceback (most recent call last): ...")),
        ]),
    )]);
    assert_eq!(
        error_message(&result),
        Some("[execution error] Traceback (most recent call last): ...".to_owned())
    );
}

#[cfg(test)]
#[test]
fn error_message_returns_none_when_the_error_object_is_empty() {
    let result = test_object([("error", json!({}))]);
    assert_eq!(error_message(&result), None);
}

#[cfg(test)]
#[test]
fn error_message_falls_back_to_unknown_error_when_no_field_is_usable() {
    let result = test_object([(
        "error",
        test_object([
            ("name", json!(null)),
            ("value", json!(null)),
            ("traceback", json!(null)),
        ]),
    )]);
    assert_eq!(
        error_message(&result),
        Some("[execution error] unknown error".to_owned())
    );
}

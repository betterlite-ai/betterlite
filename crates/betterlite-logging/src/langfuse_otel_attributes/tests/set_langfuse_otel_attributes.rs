#![cfg(test)]

use std::collections::BTreeMap;

use super::support::{NoEnvironment, attr, entry_attributes};

#[test]
fn writes_the_protocol_input_then_the_type_then_the_metadata_then_the_messages() {
    assert_eq!(
        entry_attributes(
            r#"{"messages":[{"role":"user","content":"hi"}],"litellm_params":{"metadata":{"generation_name":"planning"}}}"#,
            None,
            &NoEnvironment
        ),
        vec![
            attr(
                "langfuse.observation.input",
                r#"{"messages":[{"content":"hi","role":"user"}]}"#
            ),
            attr("langfuse.observation.type", "generation"),
            attr("langfuse.generation.name", "planning"),
            attr(
                "langfuse.observation.input",
                r#"[{"content":"hi","role":"user"}]"#
            ),
        ]
    );
}

#[test]
fn writes_the_environment_from_dynamic_params() {
    assert_eq!(
        entry_attributes(
            r#"{"standard_callback_dynamic_params":{"langfuse_environment":"production"}}"#,
            None,
            &NoEnvironment
        ),
        vec![
            attr("langfuse.observation.input", r#"{"messages":null}"#),
            attr("langfuse.observation.type", "generation"),
            attr("langfuse.environment", "production"),
        ]
    );
}

#[test]
fn writes_the_environment_from_the_lookup() {
    let environment = BTreeMap::from([(
        "LANGFUSE_TRACING_ENVIRONMENT".to_owned(),
        "staging".to_owned(),
    )]);
    assert_eq!(
        entry_attributes("{}", None, &environment),
        vec![
            attr("langfuse.observation.input", r#"{"messages":null}"#),
            attr("langfuse.observation.type", "generation"),
            attr("langfuse.environment", "staging"),
        ]
    );
}

#[test]
fn dynamic_parameters_win_over_the_lookup() {
    let environment = BTreeMap::from([(
        "LANGFUSE_TRACING_ENVIRONMENT".to_owned(),
        "staging".to_owned(),
    )]);
    let attributes = entry_attributes(
        r#"{"standard_callback_dynamic_params":{"langfuse_environment":"production"}}"#,
        None,
        &environment,
    );
    assert_eq!(
        attributes.last(),
        Some(&attr("langfuse.environment", "production"))
    );
}

#[test]
fn an_explicit_empty_environment_is_used_as_given() {
    let environment = BTreeMap::from([(
        "LANGFUSE_TRACING_ENVIRONMENT".to_owned(),
        "staging".to_owned(),
    )]);
    let attributes = entry_attributes(
        r#"{"standard_callback_dynamic_params":{"langfuse_environment":""}}"#,
        None,
        &environment,
    );
    assert_eq!(attributes.last(), Some(&attr("langfuse.environment", "")));
}

#[test]
fn an_empty_environment_variable_writes_no_environment() {
    let environment = BTreeMap::from([("LANGFUSE_TRACING_ENVIRONMENT".to_owned(), String::new())]);
    assert_eq!(
        entry_attributes("{}", None, &environment),
        vec![
            attr("langfuse.observation.input", r#"{"messages":null}"#),
            attr("langfuse.observation.type", "generation"),
        ]
    );
}

#[test]
fn writes_the_observation_output_from_the_response() {
    assert_eq!(
        entry_attributes(
            "{}",
            Some(r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}]}"#),
            &NoEnvironment
        ),
        vec![
            attr("langfuse.observation.input", r#"{"messages":null}"#),
            attr("langfuse.observation.type", "generation"),
            attr(
                "langfuse.observation.output",
                r#"{"content":"hi","role":"assistant"}"#
            ),
        ]
    );
}

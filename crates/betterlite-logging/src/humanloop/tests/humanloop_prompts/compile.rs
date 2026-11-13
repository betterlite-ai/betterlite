use std::iter;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{json, params};
use crate::humanloop::compile::{CompileError, compile_prompt};

fn template_for(content: &str) -> Vec<Json> {
    vec![Json::Object(
        iter::once(("content".to_owned(), Json::Text(content.to_owned()))).collect(),
    )]
}

fn compiled_content(content: &str, variables: &str) -> String {
    let mut template = template_for(content);
    compile_prompt(&mut template, Some(&params(variables))).expect("compiled");
    let Json::Object(message) = &template[0] else {
        panic!("message stays an object");
    };
    match message.get("content") {
        Some(Json::Text(text)) => text.clone(),
        other => panic!("content changed type: {other:?}"),
    }
}

fn compile_error(content: &str, variables: &str) -> CompileError {
    let mut template = template_for(content);
    compile_prompt(&mut template, Some(&params(variables))).expect_err("compile error")
}

#[rstest]
#[case::one_variable("Hello {{name}}", r#"{"name": "Bob"}"#, "Hello Bob")]
#[case::repeated("{{a}}-{{a}}", r#"{"a": "x"}"#, "x-x")]
#[case::two_variables("{{a}} and {{b}}", r#"{"a": "1", "b": "2"}"#, "1 and 2")]
#[case::number("age {{age}}", r#"{"age": 31}"#, "age 31")]
#[case::boolean("on: {{flag}}", r#"{"flag": true}"#, "on: true")]
#[case::null("v: {{nothing}}", r#"{"nothing": null}"#, "v: null")]
#[case::unused_variable("Hello", r#"{"name": "Bob"}"#, "Hello")]
#[case::doubled_braces_stay_literal("{{{{keep}}}}", "{}", "{keep}")]
#[case::value_is_not_reformatted("{{a}}", r#"{"a": "{{b}}"}"#, "{{b}}")]
fn placeholders_are_replaced_with_prompt_variables(
    #[case] content: &str,
    #[case] variables: &str,
    #[case] expected: &str,
) {
    assert_eq!(compiled_content(content, variables), expected);
}

#[test]
fn a_missing_variable_is_an_error_that_names_it() {
    let error = compile_error("Hello {{name}}", "{}");
    assert!(matches!(error, CompileError::MissingVariable { .. }));
    assert_eq!(error.to_string(), "prompt variable name is missing");
}

#[test]
fn absent_prompt_variables_count_as_empty() {
    let mut template = vec![json(r#"{"content": "Hello {{name}}"}"#)];
    let error = compile_prompt(&mut template, None).expect_err("variable");
    assert!(matches!(error, CompileError::MissingVariable { .. }));
    let mut plain = vec![json(r#"{"content": "Hello"}"#)];
    compile_prompt(&mut plain, None).expect("no placeholders");
    assert_eq!(plain, vec![json(r#"{"content": "Hello"}"#)]);
}

#[rstest]
#[case::lone_close("a } b")]
#[case::unclosed("a {{ b")]
#[case::unclosed_name("a {{name")]
fn an_unmatched_brace_is_an_error(#[case] content: &str) {
    let error = compile_error(content, r#"{"name": "x"}"#);
    assert!(matches!(error, CompileError::UnmatchedBrace));
}

#[rstest]
#[case::conversion("{{name!r}}", "name!r")]
#[case::spec("{{name:>5}}", "name:>5")]
#[case::attribute("{{name.first}}", "name.first")]
#[case::index("{{name[0]}}", "name[0]")]
fn a_format_specifier_is_an_error_that_names_the_field(#[case] content: &str, #[case] field: &str) {
    let error = compile_error(content, r#"{"name": "x"}"#);
    assert!(matches!(error, CompileError::UnsupportedField { .. }));
    assert_eq!(
        error.to_string(),
        format!("prompt template field {field} uses an unsupported format specifier")
    );
}

#[rstest]
#[case::list(r#"{"a": [1]}"#)]
#[case::object(r#"{"a": {"b": 1}}"#)]
fn a_list_or_object_variable_is_an_error_that_names_it(#[case] variables: &str) {
    let error = compile_error("{{a}}", variables);
    assert_eq!(
        error.to_string(),
        "prompt variable a must be a string, number, bool or null"
    );
}

#[test]
fn only_non_empty_string_content_is_compiled() {
    let mut template = vec![
        json(r#"{"role": "user", "content": ""}"#),
        json(r#"{"role": "user"}"#),
        json(r#"{"role": "user", "content": [{"type": "text", "text": "{{name}}"}]}"#),
        json(r#"{"role": "user", "content": "{{name}}", "name": "{{name}}"}"#),
    ];
    compile_prompt(&mut template, Some(&params(r#"{"name": "Bob"}"#))).expect("compiled");
    assert_eq!(
        template,
        vec![
            json(r#"{"role": "user", "content": ""}"#),
            json(r#"{"role": "user"}"#),
            json(r#"{"role": "user", "content": [{"type": "text", "text": "{{name}}"}]}"#),
            json(r#"{"role": "user", "content": "Bob", "name": "{{name}}"}"#),
        ]
    );
}

#[test]
fn a_template_message_that_is_not_an_object_is_an_error() {
    let mut template = vec![json(r#"{"content": "ok"}"#), json(r#""just text""#)];
    let error = compile_prompt(&mut template, None).expect_err("message");
    assert_eq!(
        error.to_string(),
        "prompt template message 1 is not an object"
    );
}

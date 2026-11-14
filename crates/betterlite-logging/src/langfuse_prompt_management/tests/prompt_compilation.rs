use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::PromptParams;

use super::support;
use crate::langfuse_rest::records::Prompt;

fn variables(source: &str) -> PromptParams {
    serde_json::from_str(source).expect("the variables parse")
}

fn prompt(source: &str) -> Prompt {
    serde_json::from_str(source).expect("the prompt parses")
}

#[test]
fn a_text_prompt_becomes_one_system_message() {
    let compiled = super::super::prompt::compile_prompt(
        &prompt(&support::text_prompt()),
        Some(&variables(r#"{"role":"tutor"}"#)),
    )
    .expect("the prompt compiles");
    assert_eq!(
        compiled,
        vec![Json::Object(indexmap::IndexMap::from([
            ("role".to_owned(), Json::Text("system".to_owned())),
            (
                "content".to_owned(),
                Json::Text("You are a tutor.".to_owned())
            ),
        ]))]
    );
}

#[test]
fn a_chat_prompt_keeps_its_messages_and_expands_placeholders() {
    let compiled = super::super::prompt::compile_prompt(
        &prompt(&support::chat_prompt()),
        Some(&variables(
            r#"{"name":"Ada","history":[{"role":"user","content":"hi there"}]}"#,
        )),
    )
    .expect("the prompt compiles");
    assert_eq!(compiled.len(), 2);
    assert_eq!(
        compiled[0],
        Json::Object(indexmap::IndexMap::from([
            ("role".to_owned(), Json::Text("system".to_owned())),
            ("content".to_owned(), Json::Text("Hello Ada".to_owned())),
        ]))
    );
    assert_eq!(
        compiled[1],
        Json::Object(indexmap::IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("hi there".to_owned())),
        ]))
    );
}

#[test]
fn an_unresolved_variable_stays_verbatim() {
    let compiled = super::super::template::compile_template(
        "Hi {{name}} {{missing}}",
        &variables(r#"{"name":"Ada"}"#),
    );
    assert_eq!(compiled, "Hi Ada {{missing}}");
}

#[test]
fn a_non_string_variable_is_substituted_as_its_json_text() {
    let compiled = super::super::template::compile_template(
        "{{count}} {{items}}",
        &variables(r#"{"count":3,"items":[1,2]}"#),
    );
    assert_eq!(compiled, "3 [1,2]");
}

#[test]
fn a_placeholder_without_a_variable_becomes_a_placeholder_message() {
    let compiled = super::super::prompt::compile_prompt(
        &prompt(&support::chat_prompt()),
        Some(&variables(r#"{"name":"Ada"}"#)),
    )
    .expect("the prompt compiles");
    assert_eq!(compiled.len(), 2);
    assert_eq!(
        compiled[1],
        Json::Object(indexmap::IndexMap::from([
            ("type".to_owned(), Json::Text("placeholder".to_owned())),
            ("name".to_owned(), Json::Text("history".to_owned())),
        ]))
    );
}

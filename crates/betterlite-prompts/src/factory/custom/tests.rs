#![cfg(test)]

use super::{
    CustomPromptTemplate, RoleTemplate, custom_prompt, default_response_schema_prompt,
    function_call_prompt, response_schema_prompt,
};
use crate::builtin::{Content, Field, Message};
use crate::error::FactoryError;
use rstest::rstest;
use std::collections::BTreeMap;

fn template() -> CustomPromptTemplate {
    let mut roles = BTreeMap::new();
    roles.insert(
        "system".to_owned(),
        RoleTemplate {
            pre_message: "<<SYS>>".to_owned(),
            post_message: "<</SYS>>".to_owned(),
        },
    );
    roles.insert(
        "user".to_owned(),
        RoleTemplate {
            pre_message: "[INST] ".to_owned(),
            post_message: " [/INST]".to_owned(),
        },
    );
    CustomPromptTemplate {
        roles,
        initial_prompt_value: "<s>".to_owned(),
        final_prompt_value: "</s>".to_owned(),
        bos_token: "<bos>".to_owned(),
        eos_token: "<eos>".to_owned(),
    }
}

fn messages(json_text: &str) -> Vec<Message> {
    serde_json::from_str(json_text).unwrap()
}

fn field(json_text: &str) -> Field {
    serde_json::from_str(json_text).unwrap()
}

#[rstest]
#[case::custom_prompt_roles(
    r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#,
    "<bos><s><<SYS>>s<</SYS>>[INST] u [/INST]a<eos></s>"
)]
#[case::custom_prompt_reopens_bos_after_assistant(
    r#"[{"role":"assistant","content":"a"},{"role":"system","content":"s"}]"#,
    "<bos><s>a<eos><bos><<SYS>>s<</SYS>></s>"
)]
#[case::custom_prompt_unmapped_role_has_no_affixes(
    r#"[{"role":"other","content":"x"}]"#,
    "<bos><s>x</s>"
)]
#[case::custom_prompt_list_content_joins_text_parts(
    r#"[{"role":"user","content":[{"type":"text","text":"t"},{"type":"image_url","image_url":"raw"}]}]"#,
    "<bos><s>[INST] t [/INST]</s>"
)]
fn prompt_templates_prompt_template_tests_behavior(#[case] wire: &str, #[case] expected: &str) {
    let rendered = custom_prompt(&template(), &messages(wire)).unwrap();
    assert_eq!(rendered, expected);
}

#[test]
fn custom_prompt_skips_null_content() {
    let rendered = custom_prompt(
        &template(),
        &messages(r#"[{"role":"user","content":null}]"#),
    )
    .unwrap();
    assert_eq!(rendered, "<bos><s></s>");
}

#[test]
fn function_call_prompt_appends_to_existing_system_message() {
    let input = messages(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#);
    let functions = vec![field(r#"{"name": "lookup"}"#)];
    let output = function_call_prompt(&input, &functions).unwrap();
    let Some(Content::Text(text)) = output.first().map(|message| &message.content) else {
        panic!("expected text content");
    };
    assert!(text.starts_with("s Produce JSON OUTPUT ONLY!"));
    assert!(text.contains("{'name': 'lookup'}"));
}

#[test]
fn function_call_prompt_adds_system_message_when_absent() {
    let input = messages(r#"[{"role":"user","content":"u"}]"#);
    let output = function_call_prompt(&input, &[]).unwrap();
    assert_eq!(output.len(), 2);
    let Content::Text(text) = &output[1].content else {
        panic!("expected text content");
    };
    assert!(text.starts_with("Produce JSON OUTPUT ONLY!"));
}

#[test]
fn function_call_prompt_rejects_null_system_content() {
    let input = messages(r#"[{"role":"system","content":null}]"#);
    let error = function_call_prompt(&input, &[]).unwrap_err();
    assert!(matches!(error, FactoryError::Append { index: 0 }));
}

#[test]
fn response_schema_prompt_uses_the_custom_template_when_given() {
    let schema = field(r#"{"type": "object"}"#);
    let rendered = response_schema_prompt(&schema, Some(&template())).unwrap();
    assert_eq!(rendered, "<s>[INST] {'type': 'object'} [/INST]</s>");
}

#[test]
fn response_schema_prompt_falls_back_to_the_default_prompt() {
    let schema = field(r#"{"type": "object"}"#);
    let rendered = response_schema_prompt(&schema, None).unwrap();
    assert_eq!(rendered, default_response_schema_prompt(&schema).unwrap());
}

#[test]
fn default_response_schema_prompt_formats_the_schema_as_python_repr() {
    let schema = field(r#"{"type": "string"}"#);
    let rendered = default_response_schema_prompt(&schema).unwrap();
    assert_eq!(
        rendered,
        "Use this JSON schema: \n    ```json \n    {'type': 'string'}\n    ```"
    );
}

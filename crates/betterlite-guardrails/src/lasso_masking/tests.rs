#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::wire::ToolArguments;
use crate::bedrock_message_selection::wire::{Content, Role};
use crate::lasso::wire::MessageType;

use super::expand::expand_for_classification;
use super::lifecycle::{self, Endpoint};
use super::map_back::map_masked_messages_back;
use super::wire::{
    LassoContent, LassoMessage, Message, ModelResponseShape, ResponseChoice, ResponseMessage,
    ToolCall, ToolCallFunction, ToolUseBlock,
};

fn tool_arguments(json: &str) -> ToolArguments {
    ToolArguments::Parsed(serde_json::from_str(json).expect("valid tool arguments"))
}

fn assistant_tool_call_message(id: &str, name: &str, arguments: &str) -> Message {
    Message {
        role: Role::Assistant,
        content: None,
        tool_calls: Some(vec![ToolCall {
            id: Some(id.to_owned()),
            function: Some(ToolCallFunction {
                name: Some(name.to_owned()),
                arguments: Some(arguments.to_owned()),
                extra: BTreeMap::new(),
            }),
            extra: BTreeMap::new(),
        }]),
        tool_call_id: None,
        extra: BTreeMap::new(),
    }
}

#[test]
fn lasso_masking_behaviors() {
    let messages = vec![assistant_tool_call_message(
        "call",
        "lookup",
        r#"{"email":"a"}"#,
    )];

    let expanded = expand_for_classification(&messages);
    let LassoContent::ToolUse(block) = &expanded[0].content else {
        panic!("expected a tool_use block");
    };
    assert_eq!(block.name.as_deref(), Some("lookup"));

    let masked = vec![LassoMessage {
        role: "model".to_owned(),
        content: LassoContent::ToolUse(ToolUseBlock {
            kind: "tool_use".to_owned(),
            id: Some("call".to_owned()),
            name: Some("lookup".to_owned()),
            input: Some(tool_arguments(r#"{"email":"[MASKED EMAIL]"}"#)),
        }),
    }];

    let mapped = map_masked_messages_back(&messages, &masked);
    let arguments = mapped[0].tool_calls.as_ref().unwrap()[0]
        .function
        .as_ref()
        .unwrap()
        .arguments
        .as_deref();
    assert_eq!(arguments, Some(r#"{"email":"[MASKED EMAIL]"}"#));
}

#[test]
fn lasso_masking_prompt_lifecycle() {
    let original = vec![Message {
        role: Role::User,
        content: Some(Content::Text("secret".to_owned())),
        tool_calls: None,
        tool_call_id: None,
        extra: BTreeMap::new(),
    }];
    let masked = vec![LassoMessage {
        role: "user".to_owned(),
        content: LassoContent::Text("[MASKED]".to_owned()),
    }];

    let mapped = map_masked_messages_back(&original, &masked);
    let Some(Content::Text(text)) = &mapped[0].content else {
        panic!("expected masked text content");
    };
    assert_eq!(text, "[MASKED]");

    let outcome = lifecycle::run(
        &original,
        Some("input secret"),
        true,
        Some(&[
            LassoMessage {
                role: "user".to_owned(),
                content: LassoContent::Text("[MASKED]".to_owned()),
            },
            LassoMessage {
                role: "user".to_owned(),
                content: LassoContent::Text("[MASKED INPUT]".to_owned()),
            },
        ]),
    );
    assert_eq!(outcome.endpoint, Endpoint::Classifix);
    assert_eq!(outcome.input.as_deref(), Some("[MASKED INPUT]"));
}

fn response_with_tool_call() -> ModelResponseShape {
    ModelResponseShape {
        choices: vec![ResponseChoice {
            message: Some(ResponseMessage {
                role: Role::Assistant,
                content: Some("secret".to_owned()),
                tool_calls: Some(vec![ToolCall {
                    id: Some("call".to_owned()),
                    function: Some(ToolCallFunction {
                        name: Some("lookup".to_owned()),
                        arguments: Some("{}".to_owned()),
                        extra: BTreeMap::new(),
                    }),
                    extra: BTreeMap::new(),
                }]),
                extra: BTreeMap::new(),
            }),
            extra: BTreeMap::new(),
        }],
        extra: BTreeMap::new(),
    }
}

#[rstest::rstest]
#[case::masking(true, Endpoint::Classifix, "[MASKED]", r#"{"email":"[MASKED EMAIL]"}"#)]
#[case::classification(false, Endpoint::Classify, "secret", "{}")]
fn lasso_masking_lifecycle(
    #[case] mask: bool,
    #[case] endpoint: Endpoint,
    #[case] content: &str,
    #[case] tool_input: &str,
) {
    let response = response_with_tool_call();
    let masked: Vec<LassoMessage> = serde_json::from_str(r#"[
        {"role":"assistant","content":"[MASKED]"},
        {"role":"model","content":{"type":"tool_use","id":"call","name":"lookup","input":{"email":"[MASKED EMAIL]"}}}
    ]"#).unwrap();

    let original = serde_json::to_value(&response).unwrap();
    let outcome = lifecycle::post_call(&response, mask, Some(&masked));
    assert_eq!(outcome.message_type, MessageType::Completion);
    assert_eq!(outcome.endpoint, endpoint);
    assert_eq!(outcome.inspected.len(), 2);
    assert!(matches!(
        outcome.inspected[1].content,
        LassoContent::ToolUse(_)
    ));
    let updated = outcome.response;
    let message = updated.choices[0].message.as_ref().unwrap();
    assert_eq!(message.content.as_deref(), Some(content));
    let arguments = message.tool_calls.as_ref().unwrap()[0]
        .function
        .as_ref()
        .unwrap()
        .arguments
        .as_deref();
    assert_eq!(arguments, Some(tool_input));
    assert_eq!(serde_json::to_value(&response).unwrap(), original);
}

#[test]
fn lasso_masking_text_count_mismatch_skips_rewrite() {
    let original = vec![
        Message {
            role: Role::User,
            content: Some(Content::Text("first secret".to_owned())),
            tool_calls: None,
            tool_call_id: None,
            extra: BTreeMap::new(),
        },
        Message {
            role: Role::User,
            content: Some(Content::Text("second secret".to_owned())),
            tool_calls: None,
            tool_call_id: None,
            extra: BTreeMap::new(),
        },
        assistant_tool_call_message("call", "lookup", r#"{"email":"a"}"#),
    ];
    let masked = vec![LassoMessage {
        role: "user".to_owned(),
        content: LassoContent::Text("[MASKED]".to_owned()),
    }];

    let mapped = map_masked_messages_back(&original, &masked);
    let Some(Content::Text(text)) = &mapped[0].content else {
        panic!("expected the original text content");
    };
    assert_eq!(
        text, "first secret",
        "a text-count mismatch must leave content untouched"
    );
    assert!(
        mapped[2].tool_calls.is_some(),
        "tool_calls still rewrite even when the text rewrite is skipped"
    );
}

#[test]
fn post_call_preserves_choices_and_metadata_on_count_mismatch() {
    let response: ModelResponseShape = serde_json::from_str(r#"{
        "id":"response", "choices":[
            {"index":0,"message":{"role":"assistant","content":"first","reasoning_content":"reasoning"}},
            {"index":1,"message":{"role":"assistant","content":null,"tool_calls":[
                {"id":"call","type":"function","function":{"name":"lookup","arguments":"{}"}}
            ]}},
            {"index":2,"message":{"role":"assistant","content":"last"}}
        ]}"#).unwrap();
    let masked: Vec<LassoMessage> = serde_json::from_str(
        r#"[
        {"role":"assistant","content":"too few texts"},
        {"role":"model","content":{"type":"tool_use","id":"call","name":"lookup","input":"masked"}}
    ]"#,
    )
    .unwrap();
    let outcome = lifecycle::post_call(&response, true, Some(&masked));
    assert_eq!(outcome.inspected.len(), 3);
    let mut expected = serde_json::to_value(&response).unwrap();
    expected["choices"][1]["message"]["tool_calls"][0]["function"]["arguments"] =
        serde_json::json!(r#""masked""#);
    assert_eq!(serde_json::to_value(outcome.response).unwrap(), expected);
}

#[test]
fn post_call_without_masked_output_preserves_response() {
    let response = response_with_tool_call();
    let outcome = lifecycle::post_call(&response, true, None);
    assert_eq!(outcome.endpoint, Endpoint::Classifix);
    assert_eq!(outcome.inspected.len(), 2);
    assert_eq!(
        serde_json::to_value(outcome.response).unwrap(),
        serde_json::to_value(response).unwrap()
    );
}

#[test]
fn lasso_wire_replays_corpus_and_preserves_nulls() {
    replay_lasso_corpus();
}

fn replay_lasso_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/lasso_wire");
    for entry in fs::read_dir(corpus).expect("corpus") {
        let bytes = fs::read(entry.expect("entry").path()).expect("seed");
        let messages: Vec<Message> = serde_json::from_slice(&bytes).expect("messages");
        let encoded = serde_json::to_string(&messages).expect("encoded");
        assert!(encoded.contains("\"content\":null"));
    }
}

#[test]
fn lasso_tool_inputs_serialize_strings_and_arrays_directly() {
    assert_direct_tool_input(r#""masked""#);
    assert_direct_tool_input(r#"["masked"]"#);
}

fn assert_direct_tool_input(input: &str) {
    let block: LassoContent =
        serde_json::from_str(&format!(r#"{{"type":"tool_use","input":{input}}}"#))
            .expect("tool block");
    let encoded = serde_json::to_value(block).expect("encoded");
    assert_eq!(encoded["input"].to_string(), input);
}

#![cfg(test)]

use std::collections::BTreeMap;

use crate::bedrock_message_selection::wire::{Content, Role};
use crate::lasso::wire::ToolArguments;

use super::apply_response::mask_model_response;
use super::expand::expand_for_classification;
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
fn lasso_masking_lifecycle() {
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
                        name: None,
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

#[test]
fn lasso_masking_response_rewrites_text_and_tool_arguments() {
    let response = response_with_tool_call();
    let masked = vec![
        LassoMessage {
            role: "assistant".to_owned(),
            content: LassoContent::Text("[MASKED]".to_owned()),
        },
        LassoMessage {
            role: "model".to_owned(),
            content: LassoContent::ToolUse(ToolUseBlock {
                kind: "tool_use".to_owned(),
                id: Some("call".to_owned()),
                name: None,
                input: Some(tool_arguments(r#"{"email":"[MASKED EMAIL]"}"#)),
            }),
        },
    ];

    let updated = mask_model_response(&response, &masked);
    let message = updated.choices[0].message.as_ref().unwrap();
    assert_eq!(message.content.as_deref(), Some("[MASKED]"));
    let arguments = message.tool_calls.as_ref().unwrap()[0]
        .function
        .as_ref()
        .unwrap()
        .arguments
        .as_deref();
    assert_eq!(arguments, Some(r#"{"email":"[MASKED EMAIL]"}"#));
}

#![cfg(test)]

use crate::qwen_deepseek::prompt::{PromptError, convert_messages_to_prompt};
use crate::qwen_deepseek::tests::support::prompts;

pub(in crate::qwen_deepseek::tests) fn qwen3_chat_prompt() {
    empty_input_keeps_the_start();
    every_role_block();
    missing_and_null_content();
    unknown_roles_are_omitted();
    user_lists_concatenate();
    non_user_lists_keep_python_display();
    malformed_values_fail();
}

fn empty_input_keeps_the_start() {
    let prompt = convert_messages_to_prompt(&[]).expect("the empty prompt converts");
    assert_eq!(prompt, "<|im_start|>assistant\n");
}

fn every_role_block() {
    let messages = prompts(
        r#"[
            {"role": "system", "content": "sys"},
            {"role": "user", "content": "usr"},
            {"role": "assistant", "content": "asst"},
            {"role": "tool", "content": "tool out"}
        ]"#,
    );
    let prompt = convert_messages_to_prompt(&messages).expect("the prompt converts");
    assert_eq!(
        prompt,
        "<|im_start|>system\nsys<|im_end|>\n<|im_start|>user\nusr<|im_end|>\n<|im_start|>assistant\nasst<|im_end|>\n<|im_start|>tool\ntool out<|im_end|>\n<|im_start|>assistant\n"
    );
    let single = prompts(r#"[{"role": "system", "content": "only"}]"#);
    assert_eq!(
        convert_messages_to_prompt(&single).expect("the prompt converts"),
        "<|im_start|>system\nonly<|im_end|>\n<|im_start|>assistant\n",
        "blocks are joined with one newline"
    );
}

fn missing_and_null_content() {
    let messages = prompts(
        r#"[
            {"role": "user"},
            {"role": "system", "content": null},
            {"role": "tool", "content": null}
        ]"#,
    );
    assert_eq!(
        convert_messages_to_prompt(&messages).expect("the prompt converts"),
        "<|im_start|>user\n<|im_end|>\n<|im_start|>system\nNone<|im_end|>\n<|im_start|>tool\nNone<|im_end|>\n<|im_start|>assistant\n"
    );
}

fn unknown_roles_are_omitted() {
    let messages = prompts(
        r#"[
            {"role": "developer", "content": "x"},
            {"content": "no role"},
            {"role": null, "content": "null role"},
            {"role": 5, "content": "numeric role"},
            {"role": "Assistant", "content": "wrong case"}
        ]"#,
    );
    assert_eq!(
        convert_messages_to_prompt(&messages).expect("the prompt converts"),
        "<|im_start|>assistant\n"
    );
}

fn user_lists_concatenate() {
    let messages = prompts(
        r#"[{"role": "user", "content": [
            {"type": "text", "text": "a"},
            {"type": "text"},
            {"type": "image_url", "image_url": {"url": "https://example.invalid/cat.png"}},
            {"type": "audio", "audio": {"data": "ignored"}},
            {"type": "text", "text": "b"}
        ]}]"#,
    );
    assert_eq!(
        convert_messages_to_prompt(&messages).expect("the prompt converts"),
        "<|im_start|>user\na<|vision_start|><|image_pad|><|vision_end|>b<|im_end|>\n<|im_start|>assistant\n",
        "text items concatenate, images become the placeholder and other kinds vanish"
    );
}

fn non_user_lists_keep_python_display() {
    let messages = prompts(
        r#"[
            {"role": "system", "content": [{"a": 1}, "x"]},
            {"role": "tool", "content": [true, null]}
        ]"#,
    );
    assert_eq!(
        convert_messages_to_prompt(&messages).expect("the prompt converts"),
        "<|im_start|>system\n[{'a': 1}, 'x']<|im_end|>\n<|im_start|>tool\n[True, None]<|im_end|>\n<|im_start|>assistant\n"
    );
}

fn malformed_values_fail() {
    message_and_item_errors();
    tool_call_errors();
}

fn message_and_item_errors() {
    let not_a_message = prompts(r#"["hello"]"#);
    assert!(matches!(
        convert_messages_to_prompt(&not_a_message).expect_err("a string message fails"),
        PromptError::MessageNotObject { index } if index == 0
    ));
    let not_an_item = prompts(r#"[{"role": "user", "content": [1]}]"#);
    assert!(matches!(
        convert_messages_to_prompt(&not_an_item).expect_err("a numeric item fails"),
        PromptError::ContentItemNotObject { index, item } if index == 0 && item == 0
    ));
    let not_text = prompts(
        r#"[{"role": "user", "content": [{"type": "text", "text": null}, {"type": "text", "text": 5}]}]"#,
    );
    assert!(matches!(
        convert_messages_to_prompt(&not_text).expect_err("a null text fails"),
        PromptError::ContentItemText { index, item } if index == 0 && item == 0
    ));
}

fn tool_call_errors() {
    let not_a_call = prompts(r#"[{"role": "assistant", "tool_calls": [7]}]"#);
    assert!(matches!(
        convert_messages_to_prompt(&not_a_call).expect_err("a numeric tool call fails"),
        PromptError::ToolCallNotObject { index, call } if index == 0 && call == 0
    ));
    for function in [
        r#"{"function": null}"#,
        r#"{"function": 5}"#,
        r#"{"function": []}"#,
    ] {
        let source = format!(r#"[{{"role": "assistant", "tool_calls": [{function}]}}]"#);
        assert!(
            matches!(
                convert_messages_to_prompt(&prompts(&source)).expect_err("a non-object function fails"),
                PromptError::FunctionNotObject { index, call } if index == 0 && call == 0
            ),
            "{function}"
        );
    }
}

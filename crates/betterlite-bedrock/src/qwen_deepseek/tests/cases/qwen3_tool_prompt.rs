#![cfg(test)]

use crate::qwen_deepseek::prompt::convert_messages_to_prompt;
use crate::qwen_deepseek::tests::support::prompts;

pub(in crate::qwen_deepseek::tests) fn qwen3_tool_prompt() {
    arguments_are_interpolated_raw();
    missing_pieces_default_empty();
    ordinary_assistant_blocks();
}

fn arguments_are_interpolated_raw() {
    let messages = prompts(
        r#"[{"role": "assistant", "content": "ignored", "tool_calls": [
            {"id": "call-1", "type": "function", "function": {"name": "read", "arguments": "{\"path\": \"a\"}"}},
            {"id": "call-2", "type": "function", "function": {"name": "write", "arguments": "say \"hi\""}}
        ]}]"#,
    );
    let prompt = convert_messages_to_prompt(&messages).expect("the tool prompt converts");
    let expected = concat!(
        "<|im_start|>assistant\n<tool_call>\n",
        "{\"name\": \"read\", \"arguments\": \"{\"path\": \"a\"}\"}\n",
        "</tool_call><|im_end|>\n",
        "<|im_start|>assistant\n<tool_call>\n",
        "{\"name\": \"write\", \"arguments\": \"say \"hi\"\"}\n",
        "</tool_call><|im_end|>\n",
        "<|im_start|>assistant\n"
    );
    assert_eq!(prompt, expected);
    assert!(
        !prompt.contains("ignored"),
        "a tool-call assistant omits its content"
    );
}

fn missing_pieces_default_empty() {
    for (call, name, arguments) in [
        (r#"{}"#, "", ""),
        (r#"{"function": {}}"#, "", ""),
        (r#"{"function": {"name": "only"}}"#, "only", ""),
        (
            r#"{"function": {"name": "numbers", "arguments": 12}}"#,
            "numbers",
            "12",
        ),
        (
            r#"{"function": {"name": null, "arguments": true}}"#,
            "None",
            "True",
        ),
        (
            r#"{"function": {"name": "float", "arguments": 5.0}}"#,
            "float",
            "5.0",
        ),
    ] {
        let source = format!(r#"[{{"role": "assistant", "tool_calls": [{call}]}}]"#);
        let prompt =
            convert_messages_to_prompt(&prompts(&source)).expect("the tool prompt converts");
        let expected = format!(
            "<|im_start|>assistant\n<tool_call>\n{{\"name\": \"{name}\", \"arguments\": \"{arguments}\"}}\n</tool_call><|im_end|>\n<|im_start|>assistant\n"
        );
        assert_eq!(prompt, expected, "{call}");
    }
}

fn ordinary_assistant_blocks() {
    let expected = "<|im_start|>assistant\nplain<|im_end|>\n<|im_start|>assistant\n";
    for tool_calls in ["[]", "null", "{}", "\"x\"", "5", "false"] {
        let source =
            format!(r#"[{{"role": "assistant", "content": "plain", "tool_calls": {tool_calls}}}]"#);
        let prompt =
            convert_messages_to_prompt(&prompts(&source)).expect("the assistant prompt converts");
        assert_eq!(prompt, expected, "tool_calls {tool_calls}");
    }
    let missing = prompts(r#"[{"role": "assistant", "content": "plain"}]"#);
    assert_eq!(
        convert_messages_to_prompt(&missing).expect("the assistant prompt converts"),
        expected,
        "a missing tool_calls key is the ordinary block"
    );
}

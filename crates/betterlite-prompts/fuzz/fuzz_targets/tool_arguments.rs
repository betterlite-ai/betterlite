#![no_main]

use betterlite_prompts::builtin::{Content, Message, ToolCall, ToolFunction, ollama::ollama};
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    if let Ok(arguments) = str::from_utf8(bytes) {
        let messages = [Message {
            role: "assistant".to_owned(),
            content: Content::Text(String::new()),
            tool_calls: vec![ToolCall {
                id: "id".to_owned(),
                kind: "function".to_owned(),
                function: ToolFunction {
                    name: "f".to_owned(),
                    arguments: arguments.to_owned(),
                },
            }],
        }];
        drop(ollama("model", &messages));
    }
});

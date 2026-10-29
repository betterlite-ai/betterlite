#![no_main]

use betterlite_prompts::builtin::{Calls, Content, Message};
use betterlite_prompts::jinja::{LocalTemplate, hf_chat_template};
use chrono::{TimeZone, Utc};
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    if let Ok(chat_template) = str::from_utf8(bytes) {
        let template = LocalTemplate {
            chat_template: chat_template.to_owned(),
            bos_token: None,
            eos_token: None,
        };
        let messages = [Message {
            role: "user".to_owned(),
            content: Content::Text("hi".to_owned()),
            tool_calls: Calls::Absent,
            extra: Default::default(),
        }];
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        drop(hf_chat_template(&template, &messages, now));
    }
});

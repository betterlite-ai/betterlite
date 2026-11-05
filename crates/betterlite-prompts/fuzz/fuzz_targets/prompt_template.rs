#![no_main]

use betterlite_prompts::builtin::{Calls, Content, Message};
use betterlite_prompts::jinja::{LocalTemplate, hf_chat_template};
use chrono::{TimeZone as _, Utc};
use indexmap::IndexMap;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
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
            extra: IndexMap::default(),
        }];
        if let Some(now) = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).single() {
            drop(hf_chat_template(&template, &messages, now));
        }
    }
});

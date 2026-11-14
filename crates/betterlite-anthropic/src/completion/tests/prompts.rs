#![cfg(test)]

use std::collections::BTreeMap;

use crate::completion::error::CompletionError;
use crate::completion::prompt::{CustomPromptEntry, PromptSource, prompt_from_messages};

use super::{USER_PROMPT, clock, factory, templates, user_messages};

pub(super) async fn factory_prompts() {
    let prompts = factory();
    let none = BTreeMap::new();
    let source = PromptSource {
        custom_prompt_dict: &none,
        prompts: &prompts,
        now: clock(),
    };
    let messages = user_messages();
    let built = prompt_from_messages(&source, "claude-2", &messages).await;
    assert_eq!(built.unwrap(), USER_PROMPT);
    let instant = prompt_from_messages(&source, "claude-instant-1", &messages).await;
    assert_eq!(instant.unwrap(), USER_PROMPT);
    let unsupported = prompt_from_messages(&source, "claude-3", &messages).await;
    assert!(matches!(unsupported, Err(CompletionError::Prompt(_))));
}

pub(super) async fn custom_prompts() {
    let prompts = factory();
    let dict = templates();
    let source = PromptSource {
        custom_prompt_dict: &dict,
        prompts: &prompts,
        now: clock(),
    };
    let messages = user_messages();
    let templated = prompt_from_messages(&source, "claude-2", &messages).await;
    assert_eq!(templated.unwrap(), "<U:hello|>");
    let other = prompt_from_messages(&source, "claude-instant-1", &messages).await;
    assert_eq!(other.unwrap(), USER_PROMPT);
}

pub(super) fn custom_entries_require_every_key() {
    for incomplete in [
        r#"{"roles":{},"initial_prompt_value":""}"#,
        r#"{"roles":{},"final_prompt_value":""}"#,
        r#"{"initial_prompt_value":"","final_prompt_value":""}"#,
    ] {
        assert!(serde_json::from_str::<CustomPromptEntry>(incomplete).is_err());
    }
}

#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::langfuse_rest::records::{Prompt, PromptMessage};

use super::prompts::attach_generation_prompt;
use super::support::{idle_client, metadata, prompt_client, prompt_value};

const TEXT_PROMPT: &str = r#"{
    "name": "greeting",
    "version": 3,
    "prompt": "Hello {{name}}",
    "config": {"model": "gpt-4o"},
    "labels": ["production"],
    "tags": ["greeting"]
}"#;

const CHAT_PROMPT: &str = r#"{
    "name": "greeting",
    "version": 5,
    "prompt": [{"role": "user", "content": "hi"}, {"name": "history"}],
    "config": null,
    "labels": [],
    "tags": []
}"#;

const NUMBER_PROMPT: &str = r#"{"name": "greeting", "version": 3, "prompt": 7}"#;
const NAMELESS_PROMPT: &str = r#"{"version": 3, "prompt": "Hello"}"#;
const FRACTIONAL_VERSION: &str = r#"{"name": "greeting", "version": 3.5, "prompt": "Hello"}"#;
const VERSIONLESS_PROMPT: &str = r#"{"name": "greeting", "prompt": "Hello"}"#;

#[tokio::test]
async fn prompt_version_text_content_builds_a_text_prompt_client() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(TEXT_PROMPT));
    let attachment = attach_generation_prompt(&mut clean, None, &client).await;
    let Some(Prompt::Text(text)) = prompt_client(attachment) else {
        panic!("a text prompt client")
    };
    assert_eq!(text.meta.name, "greeting");
    assert_eq!(text.meta.version, 3);
    assert_eq!(text.meta.config, prompt_value(r#"{"model": "gpt-4o"}"#));
    assert_eq!(text.meta.labels, vec!["production".to_owned()]);
    assert_eq!(text.meta.tags, vec!["greeting".to_owned()]);
}

#[tokio::test]
async fn prompt_version_chat_content_builds_a_chat_prompt_client() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(CHAT_PROMPT));
    let attachment = attach_generation_prompt(&mut clean, None, &client).await;
    let Some(Prompt::Chat(chat)) = prompt_client(attachment) else {
        panic!("a chat prompt client")
    };
    assert_eq!(chat.meta.version, 5);
    assert_eq!(chat.meta.config, Json::Null);
    assert_eq!(chat.prompt.len(), 2);
    assert!(matches!(chat.prompt.first(), Some(PromptMessage::Chat(_))));
    assert!(matches!(
        chat.prompt.get(1),
        Some(PromptMessage::Placeholder(_))
    ));
}

#[tokio::test]
async fn prompt_version_unusable_prompt_content_logs_no_prompt() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(NUMBER_PROMPT));
    assert!(
        attach_generation_prompt(&mut clean, None, &client)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn prompt_version_missing_name_logs_no_prompt() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(NAMELESS_PROMPT));
    assert!(
        attach_generation_prompt(&mut clean, None, &client)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn prompt_version_fractional_version_logs_no_prompt() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(FRACTIONAL_VERSION));
    assert!(
        attach_generation_prompt(&mut clean, None, &client)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn prompt_version_missing_version_logs_no_prompt() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(VERSIONLESS_PROMPT));
    assert!(
        attach_generation_prompt(&mut clean, None, &client)
            .await
            .is_none()
    );
}

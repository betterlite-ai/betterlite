#![cfg(test)]

use crate::langfuse_rest::records::Prompt;

use super::prompts::attach_generation_prompt;
use super::support::{idle_client, metadata, prompt_client, prompt_metadata, prompt_value};

const CHAT_PROMPT: &str = r#"{
    "type": "chat",
    "name": "greeting",
    "version": 2,
    "prompt": [{"role": "system", "content": "You help"}],
    "config": {"model": "gpt-4o"},
    "labels": ["production"],
    "tags": ["greeting"]
}"#;

const TEXT_PROMPT: &str = r#"{
    "type": "text",
    "name": "farewell",
    "version": 4,
    "prompt": "Goodbye {{name}}",
    "config": {},
    "labels": [],
    "tags": ["farewell"]
}"#;

const INCOMPLETE_CHAT_PROMPT: &str = r#"{"type": "chat", "name": "greeting"}"#;
const UNRECOGNIZED_PROMPT: &str = r#"{"prompt_name": "greeting"}"#;

#[tokio::test]
async fn prompt_type_chat_builds_a_chat_prompt_client() {
    let client = idle_client();
    let managed = prompt_metadata("managed", "langfuse");
    let mut clean = metadata(prompt_value(CHAT_PROMPT));
    let attachment = attach_generation_prompt(&mut clean, Some(&managed), &client).await;
    let Some(Prompt::Chat(chat)) = prompt_client(attachment) else {
        panic!("a chat prompt client")
    };
    assert_eq!(chat.meta.name, "greeting");
    assert_eq!(chat.meta.version, 2);
    assert_eq!(chat.meta.labels, vec!["production".to_owned()]);
    assert_eq!(chat.meta.tags, vec!["greeting".to_owned()]);
    assert_eq!(chat.prompt.len(), 1);
}

#[tokio::test]
async fn prompt_type_text_builds_a_text_prompt_client() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(TEXT_PROMPT));
    let attachment = attach_generation_prompt(&mut clean, None, &client).await;
    let Some(Prompt::Text(text)) = prompt_client(attachment) else {
        panic!("a text prompt client")
    };
    assert_eq!(text.meta.name, "farewell");
    assert_eq!(text.meta.version, 4);
    assert_eq!(text.prompt, "Goodbye {{name}}");
}

#[tokio::test]
async fn prompt_type_incomplete_dict_logs_no_prompt() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(INCOMPLETE_CHAT_PROMPT));
    let attachment = attach_generation_prompt(&mut clean, None, &client).await;
    assert!(attachment.is_none());
    assert!(!clean.contains_key("prompt"));
}

#[tokio::test]
async fn prompt_type_unrecognized_dict_logs_no_prompt() {
    let client = idle_client();
    let mut clean = metadata(prompt_value(UNRECOGNIZED_PROMPT));
    let attachment = attach_generation_prompt(&mut clean, None, &client).await;
    assert!(attachment.is_none());
    assert!(!clean.contains_key("prompt"));
}

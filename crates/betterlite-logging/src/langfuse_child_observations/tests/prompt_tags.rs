#![cfg(test)]

use crate::langfuse_rest::records::Prompt;

use super::prompts::attach_generation_prompt;
use super::support::{idle_client, metadata, prompt_client, prompt_value};

const WITH_TAGS: &str =
    r#"{"name": "greeting", "version": 3, "prompt": "Hello", "tags": ["alpha", "beta"]}"#;
const WITHOUT_TAGS: &str = r#"{"name": "greeting", "version": 3, "prompt": "Hello"}"#;
const NULL_TAGS: &str = r#"{"name": "greeting", "version": 3, "prompt": "Hello", "tags": null}"#;
const MALFORMED_TAGS: &str =
    r#"{"name": "greeting", "version": 3, "prompt": "Hello", "tags": [1]}"#;

async fn tags(definition: &str) -> Option<Vec<String>> {
    let client = idle_client();
    let mut clean = metadata(prompt_value(definition));
    match prompt_client(attach_generation_prompt(&mut clean, None, &client).await) {
        Some(Prompt::Text(text)) => Some(text.meta.tags),
        Some(Prompt::Chat(_)) | None => None,
    }
}

#[tokio::test]
async fn prompt_tags_copies_the_callers_tags() {
    assert_eq!(
        tags(WITH_TAGS).await,
        Some(vec!["alpha".to_owned(), "beta".to_owned()])
    );
}

#[tokio::test]
async fn prompt_tags_defaults_to_an_empty_list() {
    assert_eq!(tags(WITHOUT_TAGS).await, Some(Vec::new()));
    assert_eq!(tags(NULL_TAGS).await, Some(Vec::new()));
}

#[tokio::test]
async fn prompt_tags_rejects_a_non_string_entry() {
    assert_eq!(tags(MALFORMED_TAGS).await, None);
}

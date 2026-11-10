#![cfg(test)]

use crate::langfuse_rest::records::Prompt;

use super::prompts::attach_generation_prompt;
use super::support::{idle_client, metadata, prompt_client, prompt_value};

const WITH_LABELS: &str =
    r#"{"name": "greeting", "version": 3, "prompt": "Hello", "labels": ["production", "staging"]}"#;
const WITHOUT_LABELS: &str = r#"{"name": "greeting", "version": 3, "prompt": "Hello"}"#;
const NULL_LABELS: &str =
    r#"{"name": "greeting", "version": 3, "prompt": "Hello", "labels": null}"#;
const MALFORMED_LABELS: &str =
    r#"{"name": "greeting", "version": 3, "prompt": "Hello", "labels": "production"}"#;

async fn labels(definition: &str) -> Option<Vec<String>> {
    let client = idle_client();
    let mut clean = metadata(prompt_value(definition));
    match prompt_client(attach_generation_prompt(&mut clean, None, &client).await) {
        Some(Prompt::Text(text)) => Some(text.meta.labels),
        Some(Prompt::Chat(_)) | None => None,
    }
}

#[tokio::test]
async fn prompt_labels_copies_the_callers_labels() {
    assert_eq!(
        labels(WITH_LABELS).await,
        Some(vec!["production".to_owned(), "staging".to_owned()])
    );
}

#[tokio::test]
async fn prompt_labels_defaults_to_an_empty_list() {
    assert_eq!(labels(WITHOUT_LABELS).await, Some(Vec::new()));
    assert_eq!(labels(NULL_LABELS).await, Some(Vec::new()));
}

#[tokio::test]
async fn prompt_labels_rejects_a_non_list_value() {
    assert_eq!(labels(MALFORMED_LABELS).await, None);
}

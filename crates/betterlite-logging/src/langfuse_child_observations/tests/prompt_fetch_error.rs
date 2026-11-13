#![cfg(test)]

use betterlite_models::decisions::Json;

use super::prompts::attach_generation_prompt;
use super::support::{client, metadata, prompt_metadata, prompt_server};

const NOT_FOUND: &str = r#"{"message":"prompt not found"}"#;

#[tokio::test]
async fn prompt_fetch_error_leaves_the_generation_without_a_prompt() {
    let server = prompt_server(404, NOT_FOUND).await;
    let langfuse = client(server.base_url.clone());
    let managed = prompt_metadata("missing", "langfuse");
    let mut clean = metadata(Json::Text("raw prompt".to_owned()));
    let attachment = attach_generation_prompt(&mut clean, Some(&managed), &langfuse).await;
    assert!(attachment.is_none());
    assert!(!clean.contains_key("prompt"));
    assert_eq!(
        clean.get("session_id"),
        Some(&Json::Text("session-1".to_owned()))
    );
    assert_eq!(server.request_count(), 1);
}

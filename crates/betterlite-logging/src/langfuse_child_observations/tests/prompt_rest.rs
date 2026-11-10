#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::langfuse_rest::records::Prompt;

use super::prompts::attach_generation_prompt;
use super::support::{client, metadata, prompt_client, prompt_metadata, prompt_server};

const TEXT_PROMPT: &str = r#"{"type":"text","prompt":"Hello {{name}}","name":"greeting","version":3,"config":{"model":"gpt-4o"},"labels":["production"],"tags":["greeting"]}"#;

#[tokio::test]
async fn prompt_rest_fetches_the_managed_prompt_by_id() {
    let server = prompt_server(200, TEXT_PROMPT).await;
    let langfuse = client(server.base_url.clone());
    let managed = prompt_metadata("greeting", "langfuse");
    let mut clean = IndexMap::new();
    let attachment = attach_generation_prompt(&mut clean, Some(&managed), &langfuse).await;
    let Some(Prompt::Text(prompt)) = prompt_client(attachment) else {
        panic!("the fetched text prompt client")
    };
    assert_eq!(prompt.meta.name, "greeting");
    assert_eq!(prompt.meta.version, 3);
    assert_eq!(
        server.request_lines(),
        vec!["GET /api/public/v2/prompts/greeting HTTP/1.1".to_owned()]
    );
}

#[tokio::test]
async fn prompt_rest_fetches_when_the_user_prompt_is_not_a_dict() {
    let server = prompt_server(200, TEXT_PROMPT).await;
    let langfuse = client(server.base_url.clone());
    let managed = prompt_metadata("greeting", "langfuse");
    let mut clean = metadata(Json::Text("raw prompt".to_owned()));
    let attachment = attach_generation_prompt(&mut clean, Some(&managed), &langfuse).await;
    assert!(matches!(prompt_client(attachment), Some(Prompt::Text(_))));
    assert!(!clean.contains_key("prompt"));
    assert_eq!(server.request_count(), 1);
}

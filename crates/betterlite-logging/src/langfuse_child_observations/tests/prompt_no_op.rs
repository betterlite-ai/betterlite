#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::prompts::attach_generation_prompt;
use super::support::{idle_client, metadata};

#[tokio::test]
async fn prompt_no_op_writes_nothing_without_a_prompt_or_prompt_management() {
    let client = idle_client();
    let mut absent = IndexMap::new();
    assert!(
        attach_generation_prompt(&mut absent, None, &client)
            .await
            .is_none()
    );
    let mut null = metadata(Json::Null);
    assert!(
        attach_generation_prompt(&mut null, None, &client)
            .await
            .is_none()
    );
    assert!(!null.contains_key("prompt"));
    assert_eq!(
        null.get("session_id"),
        Some(&Json::Text("session-1".to_owned()))
    );
}

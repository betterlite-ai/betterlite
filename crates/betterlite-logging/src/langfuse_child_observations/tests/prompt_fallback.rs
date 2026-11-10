#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::prompts::{GenerationPrompt, attach_generation_prompt};
use super::support::{idle_client, metadata, prompt_metadata, prompt_value};

#[rstest]
#[case::a_string(r#""hello""#)]
#[case::a_number("7")]
#[case::a_list(r#"["a", "b"]"#)]
#[case::a_bool("true")]
#[tokio::test]
async fn prompt_fallback_keeps_a_raw_value(#[case] raw: &str) {
    let client = idle_client();
    let mut clean = metadata(prompt_value(raw));
    let attachment = attach_generation_prompt(&mut clean, None, &client).await;
    assert_eq!(attachment, Some(GenerationPrompt::Raw(prompt_value(raw))));
    assert!(!clean.contains_key("prompt"));
    assert_eq!(
        clean.get("session_id"),
        Some(&Json::Text("session-1".to_owned()))
    );
}

#[tokio::test]
async fn prompt_fallback_keeps_a_raw_value_without_other_integrations() {
    let client = idle_client();
    let managed = prompt_metadata("greeting", "openai");
    let mut clean = metadata(Json::Text("raw prompt".to_owned()));
    let attachment = attach_generation_prompt(&mut clean, Some(&managed), &client).await;
    assert_eq!(
        attachment,
        Some(GenerationPrompt::Raw(Json::Text("raw prompt".to_owned())))
    );
}

#[tokio::test]
async fn prompt_fallback_keeps_a_null_without_other_integrations() {
    let client = idle_client();
    let managed = prompt_metadata("greeting", "openai");
    let mut clean = metadata(Json::Null);
    let attachment = attach_generation_prompt(&mut clean, Some(&managed), &client).await;
    assert_eq!(attachment, Some(GenerationPrompt::Raw(Json::Null)));
}

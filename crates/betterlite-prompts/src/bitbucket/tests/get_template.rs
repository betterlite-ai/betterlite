use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;

use crate::support;

#[tokio::test]
async fn get_template_renders_the_loaded_template_and_returns_the_metadata() {
    let template =
        "---\nmodel: gpt-4\ntemperature: 0.5\nmax_tokens: 100\ntop_p: 0.9\n---\nHello {{ name }}";
    let server = support::Server::start(&[("greeting.prompt", 200, template)]);
    let manager = BitBucketPromptManager::new(
        support::config(&server.base_url),
        Some("greeting".to_owned()),
        support::transport(),
    );
    let variables: Object = serde_json::from_str(r#"{"name": "Ann"}"#).expect("variables");

    let (rendered, metadata) = manager
        .get_prompt_template("greeting", Some(&variables))
        .await
        .expect("template");

    assert_eq!(rendered, "Hello Ann");
    assert_eq!(metadata.get("model").and_then(Json::as_str), Some("gpt-4"));
    assert_eq!(support::number(&metadata, "temperature"), Some(0.5_f64));
    assert_eq!(support::number(&metadata, "max_tokens"), Some(100.0_f64));
    assert_eq!(support::number(&metadata, "top_p"), Some(0.9_f64));
}

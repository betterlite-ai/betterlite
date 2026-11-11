use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support;
use crate::message_utilities::prompt_cache_checks::{
    DEFAULT_MIN_CACHE_TOKENS, PromptCacheRequest, PromptCacheSettings, get_prompt_cache_min_tokens,
    is_cached_message, is_prompt_caching_valid_prompt,
};

#[rstest]
#[case::plain_message(r#"{"role": "user", "content": "hi"}"#, false)]
#[case::ephemeral_text_block(
    r#"{"role": "user", "content": [{"type": "text", "text": "t", "cache_control": {"type": "ephemeral"}}]}"#,
    true
)]
#[case::persistent_block(
    r#"{"role": "user", "content": [{"type": "text", "text": "t", "cache_control": {"type": "persistent"}}]}"#,
    false
)]
#[case::image_block(
    r#"{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "u"}, "cache_control": {"type": "ephemeral"}}]}"#,
    false
)]
#[case::message_level_cache_control(
    r#"{"role": "user", "cache_control": {"type": "ephemeral"}}"#,
    true
)]
#[case::other_message_level_type(r#"{"role": "user", "cache_control": {"type": "other"}}"#, false)]
#[case::string_cache_control(
    r#"{"role": "user", "content": [{"type": "text", "text": "t", "cache_control": "ephemeral"}]}"#,
    false
)]
#[case::null_content(r#"{"role": "user", "content": null}"#, false)]
#[case::absent_content(r#"{"role": "user"}"#, false)]
#[case::mixed_items(
    r#"{"role": "user", "content": [5, {"type": "text", "text": "t", "cache_control": {"type": "ephemeral"}}]}"#,
    true
)]
#[case::non_object_item(r#"{"role": "user", "content": ["x"]}"#, false)]
#[tokio::test]
async fn prompt_cache_detects_ephemeral_blocks(#[case] input: &str, #[case] expected: bool) {
    assert_eq!(is_cached_message(&support::value(input), false), expected);
}

#[tokio::test]
async fn prompt_cache_detection_can_be_disabled() {
    let message = support::value(
        r#"{"role": "user", "content": [{"type": "text", "text": "t", "cache_control": {"type": "ephemeral"}}]}"#,
    );
    assert!(!is_cached_message(&message, true));
    assert!(!is_cached_message(&support::value(r#"["x"]"#), false));
}

#[rstest]
#[case::default("gpt-3.5-turbo", DEFAULT_MIN_CACHE_TOKENS)]
#[case::model_entry("anthropic.claude-3-5-haiku-20241022-v1:0", 2048)]
#[case::provider_prefixed("anthropic/claude-fable-5", 512)]
#[case::bare_model("claude-fable-5", 512)]
#[case::case_insensitive("ANTHROPIC.CLAUDE-FABLE-5", 512)]
#[case::unknown("model-that-does-not-exist", DEFAULT_MIN_CACHE_TOKENS)]
#[tokio::test]
async fn prompt_cache_min_tokens_comes_from_the_catalog(
    #[case] model: &str,
    #[case] expected: u64,
) {
    let catalog = support::catalog();
    assert_eq!(
        get_prompt_cache_min_tokens(model, PromptCacheSettings::default(), &catalog),
        expected
    );
}

#[tokio::test]
async fn prompt_cache_min_tokens_prefers_the_override() {
    let catalog = support::catalog();
    let settings = PromptCacheSettings {
        minimum_token_override: Some(7),
    };
    assert_eq!(get_prompt_cache_min_tokens("gpt-4o", settings, &catalog), 7);
    assert_eq!(
        get_prompt_cache_min_tokens("model-that-does-not-exist", settings, &catalog),
        7
    );
}

fn cache_request<'request>(
    model: &'request str,
    messages: Option<&'request [Json]>,
) -> PromptCacheRequest<'request> {
    PromptCacheRequest {
        model,
        messages,
        tools: None,
        custom_llm_provider: None,
        min_token_count: None,
    }
}

#[tokio::test]
async fn prompt_cache_requires_messages_or_tools() {
    let counter = support::counter();
    let catalog = support::catalog();
    let settings = PromptCacheSettings::default();
    // a zero minimum is met by any count, so only the count error for missing messages makes this false
    let mut nothing = cache_request("gpt-4o", None);
    nothing.min_token_count = Some(0);
    assert!(!is_prompt_caching_valid_prompt(&nothing, settings, &counter, &catalog).await);
    let tools = support::values(
        r#"[{"type": "function", "function": {"name": "f", "parameters": {"type": "object"}}}]"#,
    );
    let mut with_tools = cache_request("gpt-4o", None);
    with_tools.tools = Some(&tools);
    with_tools.min_token_count = Some(1);
    // tools without messages fail the token count, like LiteLLM's token_counter
    assert!(!is_prompt_caching_valid_prompt(&with_tools, settings, &counter, &catalog).await);
}

#[tokio::test]
async fn prompt_cache_compares_the_count_with_the_minimum() {
    let counter = support::counter();
    let catalog = support::catalog();
    let messages = support::values(r#"[{"role": "user", "content": "hello world"}]"#);
    let mut below = cache_request("gpt-4o", Some(&messages));
    below.min_token_count = Some(5);
    assert!(
        is_prompt_caching_valid_prompt(&below, PromptCacheSettings::default(), &counter, &catalog)
            .await
    );
    let mut above = cache_request("gpt-4o", Some(&messages));
    above.min_token_count = Some(1000);
    assert!(
        !is_prompt_caching_valid_prompt(&above, PromptCacheSettings::default(), &counter, &catalog)
            .await
    );
}

// the catalog minimum is 4096 for openrouter/anthropic/claude-opus-4.5 and 1024 for the bare names
#[rstest]
#[case::no_provider("anthropic/claude-opus-4.5", None, true)]
#[case::provider_prefix_is_added("anthropic/claude-opus-4.5", Some("openrouter"), false)]
#[case::model_that_starts_with_the_provider_name(
    "databricks-claude-opus-4-5",
    Some("databricks"),
    true
)]
#[tokio::test]
async fn prompt_cache_prefixes_the_model_with_the_provider(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    let counter = support::counter();
    let catalog = support::catalog();
    let words = "word ".repeat(1100);
    let messages = support::values(&format!(
        r#"[{{"role": "user", "content": "{}"}}]"#,
        words.trim_end()
    ));
    let mut request = cache_request(model, Some(&messages));
    request.custom_llm_provider = provider;
    assert_eq!(
        is_prompt_caching_valid_prompt(
            &request,
            PromptCacheSettings::default(),
            &counter,
            &catalog
        )
        .await,
        expected
    );
}

#[tokio::test]
async fn prompt_cache_returns_false_when_the_token_count_fails() {
    let counter = support::counter();
    let catalog = support::catalog();
    let messages = support::values(
        r#"[{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "https://example.com/x.png"}}]}]"#,
    );
    let mut request = cache_request("gpt-4o", Some(&messages));
    request.min_token_count = Some(1);
    assert!(
        !is_prompt_caching_valid_prompt(
            &request,
            PromptCacheSettings::default(),
            &counter,
            &catalog
        )
        .await
    );
}

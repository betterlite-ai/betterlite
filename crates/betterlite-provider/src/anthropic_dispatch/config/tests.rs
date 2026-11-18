#![cfg(test)]

use super::*;

#[test]
fn the_batches_config_needs_both_substrings_in_any_order() {
    assert_eq!(
        get_anthropic_config("https://host/v1/messages/batches/x/results?after_id=1"),
        AnthropicConfigKind::AnthropicBatches
    );
    assert_eq!(
        get_anthropic_config("https://host/results/messages/batches"),
        AnthropicConfigKind::AnthropicBatches
    );
}

#[test]
fn a_url_missing_either_substring_keeps_the_chat_config() {
    assert_eq!(
        get_anthropic_config("https://host/v1/messages"),
        AnthropicConfigKind::Anthropic
    );
    assert_eq!(
        get_anthropic_config("https://host/v1/messages/batches"),
        AnthropicConfigKind::Anthropic
    );
    assert_eq!(
        get_anthropic_config("https://host/v1/results"),
        AnthropicConfigKind::Anthropic
    );
}

#[test]
fn an_unknown_provider_keeps_the_value_error() {
    let error = selected_config("claude", "not-a-provider", None).expect_err("the tag is closed");
    assert_eq!(
        error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
}

#[test]
fn a_provider_without_a_chat_config_names_the_model_and_provider() {
    let error = selected_config("claude-3", "topaz", None).expect_err("topaz has no chat config");
    assert_eq!(
        error.to_string(),
        "Provider config not found for model: claude-3 and provider: topaz"
    );
}

#[test]
fn the_shared_selection_is_preserved() {
    use betterlite_context::chat_config_selection::tags::ChatConfigTag;
    assert_eq!(
        selected_config("claude", "anthropic", Some(ChatConfigTag::Anthropic)),
        Ok(ChatConfigTag::Anthropic)
    );
    assert_eq!(
        selected_config(
            "claude",
            "anthropic_text",
            Some(ChatConfigTag::AnthropicText)
        ),
        Ok(ChatConfigTag::AnthropicText)
    );
}

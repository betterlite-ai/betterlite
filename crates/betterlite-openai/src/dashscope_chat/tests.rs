#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{ChatBasePolicy, DEFAULT_BASE, DashscopeChatConfig, TransformInput};
use crate::chat::ChatMessage;
use crate::chat_content::{Mode, RefuseDownloads};
use crate::openai_base_config::CatalogCapabilities;

const COMPLETE_DEFAULT: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn catalog(chat_models: &[&str]) -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: chat_models
            .iter()
            .map(|model| (*model).to_owned())
            .collect(),
        text_models: BTreeSet::new(),
    }
}

fn plain_config(env_base: Option<&str>, env_api_key: Option<&str>) -> DashscopeChatConfig {
    DashscopeChatConfig {
        policy: ChatBasePolicy {
            default_base: DEFAULT_BASE.to_owned(),
            env_base: env_base.map(str::to_owned),
            env_in_complete_url: false,
        },
        env_api_key: env_api_key.map(key),
    }
}

fn brand_config(env_base: Option<&str>, env_in_complete_url: bool) -> DashscopeChatConfig {
    DashscopeChatConfig {
        policy: ChatBasePolicy {
            default_base: String::from("https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
            env_base: env_base.map(str::to_owned),
            env_in_complete_url,
        },
        env_api_key: None,
    }
}

fn supported(model: &str, chat_models: &[&str]) -> Vec<&'static str> {
    DashscopeChatConfig::supported_params(model, &catalog(chat_models))
}

#[test]
fn dashscope_chat_options_append_reasoning_effort_to_the_gpt_list() {
    let params = supported("qwen-plus", &[]);
    assert_eq!(params.last(), Some(&"reasoning_effort"));
    assert!(params.contains(&"response_format"));
    assert!(!params.contains(&"user"));
    assert_eq!(
        params
            .iter()
            .filter(|name| **name == "reasoning_effort")
            .count(),
        1
    );
}

#[test]
fn dashscope_chat_options_add_user_only_for_catalog_models() {
    let params = supported("gpt-4o", &["gpt-4o"]);
    let user = params.iter().position(|name| *name == "user");
    assert_eq!(user, params.len().checked_sub(2));
    assert_eq!(params.last(), Some(&"reasoning_effort"));
}

#[rstest]
#[case::gpt_4("gpt-4")]
#[case::gpt_35_turbo_16k("gpt-3.5-turbo-16k")]
fn dashscope_chat_options_drop_response_format_for_old_gpt_models(#[case] model: &str) {
    let params = supported(model, &[model]);
    assert!(!params.contains(&"response_format"));
    assert!(params.contains(&"user"));
    assert_eq!(params.last(), Some(&"reasoning_effort"));
}

#[rstest]
#[case::absent(None, COMPLETE_DEFAULT)]
#[case::empty(Some(""), COMPLETE_DEFAULT)]
#[case::plain(Some("https://h/v1"), "https://h/v1/chat/completions")]
#[case::trailing_slash(Some("https://h/v1/"), "https://h/v1//chat/completions")]
#[case::already_complete(Some("https://h/v1/chat/completions"), "https://h/v1/chat/completions")]
#[case::complete_with_slash(
    Some("https://h/v1/chat/completions/"),
    "https://h/v1/chat/completions//chat/completions"
)]
fn dashscope_chat_url_appends_the_completions_path_without_trimming(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let config = plain_config(Some("https://env.example/v1"), None);
    assert_eq!(config.complete_url(api_base), expected);
}

#[test]
fn dashscope_chat_url_reads_the_environment_base_only_for_a_brand_policy() {
    let plain = plain_config(Some("https://env.example/v1"), None);
    assert_eq!(plain.complete_url(None), COMPLETE_DEFAULT);
    let brand = brand_config(Some("https://brand.example/v1"), true);
    assert_eq!(
        brand.complete_url(None),
        "https://brand.example/v1/chat/completions"
    );
    assert_eq!(
        brand.complete_url(Some("https://given.example/v1")),
        "https://given.example/v1/chat/completions"
    );
    let empty_env = brand_config(Some(""), true);
    assert_eq!(
        empty_env.complete_url(None),
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions"
    );
    let no_env = brand_config(None, true);
    assert_eq!(
        no_env.complete_url(None),
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions"
    );
    let brand_ignores_env = brand_config(Some("https://brand.example/v1"), false);
    assert_eq!(
        brand_ignores_env.complete_url(None),
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions"
    );
}

#[test]
fn dashscope_chat_provider_info_chains_base_and_key() {
    let config = plain_config(None, None);
    let default = config.provider_info(None, None);
    assert_eq!(default.api_base, DEFAULT_BASE);
    assert!(default.api_key.is_none());
    let from_env = plain_config(Some("https://env.example/v1"), Some("env-key"));
    let info = from_env.provider_info(None, None);
    assert_eq!(info.api_base, "https://env.example/v1");
    assert_eq!(
        info.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
    let given = from_env.provider_info(Some("https://given.example/v1"), Some(&key("given")));
    assert_eq!(given.api_base, "https://given.example/v1");
    assert_eq!(
        given.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("given")
    );
}

#[test]
fn dashscope_chat_provider_info_skips_empty_values() {
    let config = plain_config(Some(""), Some(""));
    let info = config.provider_info(Some(""), Some(&key("")));
    assert_eq!(info.api_base, DEFAULT_BASE);
    assert!(info.api_key.is_none());
    let env = plain_config(Some("https://env.example/v1"), Some("env-key"));
    let fallback = env.provider_info(Some(""), Some(&key("")));
    assert_eq!(fallback.api_base, "https://env.example/v1");
    assert_eq!(
        fallback.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
}

#[test]
fn dashscope_chat_keeps_cache_control_for_messages_and_tools() {
    let messages: Vec<ChatMessage> = serde_json::from_str(
        r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]"#,
    )
    .expect("the cached messages");
    let optional_params: Object = serde_json::from_str(
        r#"{"tools":[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{"name":{"type":"string","pattern":"\\p{L}+"}}}},"cache_control":{"type":"ephemeral"}}]}"#,
    )
    .expect("the cached tools");
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let input = TransformInput {
        model: "qwen-plus",
        messages: &messages,
        optional_params: &optional_params,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let body = plain_config(None, None)
        .transform_request(&input, Mode::Sync)
        .expect("the dashscope transform");
    let value = serde_json::to_value(body).expect("the rendered body");
    assert_eq!(
        value["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(value["tools"][0]["cache_control"]["type"], "ephemeral");
    assert_eq!(
        value["tools"][0]["function"]["parameters"]["properties"]["name"]["pattern"],
        "\\p{L}+"
    );
}

#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{
    DashscopeFamily, FamilyEnv, chat_config, chat_policy, embedding_config, embedding_identity,
    image_config, image_identity, missing_family_key_message, rerank_config, rerank_settings,
    resolve_family_api_key,
};

fn no_env() -> FamilyEnv {
    FamilyEnv {
        base: None,
        brand_key: None,
        fallback_key: None,
        rerank_base: None,
        image_base: None,
    }
}

fn env_with_keys() -> FamilyEnv {
    FamilyEnv {
        base: Some("https://env.test/v1".to_owned()),
        brand_key: Some(SecretString::from("sk-brand")),
        fallback_key: Some(SecretString::from("sk-dash")),
        rerank_base: Some("https://env.test/reranks".to_owned()),
        image_base: Some("https://env.test/image".to_owned()),
    }
}

#[rstest]
#[case::dashscope(
    DashscopeFamily::Dashscope,
    "https://dashscope.aliyuncs.com/compatible-mode/v1",
    false
)]
#[case::qwencloud(
    DashscopeFamily::QwenCloud,
    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    true
)]
#[case::qwen_ai_platform(
    DashscopeFamily::QwenAiPlatform,
    "https://dashscope.aliyuncs.com/compatible-mode/v1",
    true
)]
fn chat_policy_carries_the_family_base_and_environment_rule(
    #[case] family: DashscopeFamily,
    #[case] default_base: &str,
    #[case] env_in_complete_url: bool,
) {
    let env = FamilyEnv {
        base: Some("https://env.test/v1".to_owned()),
        ..no_env()
    };
    let policy = chat_policy(family, &env);
    assert_eq!(policy.default_base, default_base);
    assert_eq!(policy.env_base.as_deref(), Some("https://env.test/v1"));
    assert_eq!(policy.env_in_complete_url, env_in_complete_url);
}

#[rstest]
#[case::qwencloud(DashscopeFamily::QwenCloud)]
#[case::qwen_ai_platform(DashscopeFamily::QwenAiPlatform)]
fn a_brand_chat_config_falls_through_to_the_dashscope_key(#[case] family: DashscopeFamily) {
    let with_brand = FamilyEnv {
        brand_key: Some(SecretString::from("sk-brand")),
        fallback_key: Some(SecretString::from("sk-dash")),
        ..no_env()
    };
    let brand_key = chat_config(family, &with_brand).provider_info(None, None);
    assert_eq!(
        brand_key.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-brand")
    );
    let dashscope_only = FamilyEnv {
        fallback_key: Some(SecretString::from("sk-dash")),
        ..no_env()
    };
    let dashscope_key = chat_config(family, &dashscope_only).provider_info(None, None);
    assert_eq!(
        dashscope_key
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-dash")
    );
}

#[test]
fn the_plain_embedding_identity_reads_only_the_dashscope_key() {
    let identity = embedding_identity(DashscopeFamily::Dashscope, &env_with_keys());
    assert_eq!(
        identity.default_base,
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );
    assert_eq!(
        identity.missing_key_message,
        "DashScope API key is required. Set 'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
    );
    assert_eq!(identity.env_base.as_deref(), Some("https://env.test/v1"));
    assert_eq!(
        identity.env_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-dash")
    );
    assert_eq!(
        identity
            .fallback_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        None
    );
    assert!(identity.empty_key_is_set);
}

// both brand identities carry the same key values behind their different bases and messages
#[rstest]
#[case::qwencloud(
    DashscopeFamily::QwenCloud,
    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    "https://dashscope-intl.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation",
    "QwenCloud API key is required. Set 'QWENCLOUD_API_KEY' or 'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
)]
#[case::qwen_ai_platform(
    DashscopeFamily::QwenAiPlatform,
    "https://dashscope.aliyuncs.com/compatible-mode/v1",
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation",
    "Qianwen AI Platform API key is required. Set 'QWEN_AI_PLATFORM_API_KEY' or 'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
)]
fn a_brand_identity_reads_the_brand_key_before_the_dashscope_key(
    #[case] family: DashscopeFamily,
    #[case] default_base: &str,
    #[case] default_image_base: &str,
    #[case] message: &str,
) {
    let env = env_with_keys();
    let embedding = embedding_identity(family, &env);
    let image = image_identity(family, &env);
    assert_eq!(embedding.default_base, default_base);
    assert_eq!(image.default_image_base, default_image_base);
    assert_eq!(embedding.missing_key_message, message);
    assert_eq!(image.missing_key_message, message);
    assert_eq!(embedding.env_base.as_deref(), Some("https://env.test/v1"));
    assert_eq!(
        image.env_image_base.as_deref(),
        Some("https://env.test/image")
    );
    assert_eq!(
        embedding.env_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-brand")
    );
    assert_eq!(
        image.env_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-brand")
    );
    assert_eq!(
        embedding
            .fallback_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-dash")
    );
    assert_eq!(
        image.fallback_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-dash")
    );
    assert!(!embedding.empty_key_is_set);
}

#[test]
fn embedding_config_carries_the_family_identity() {
    let config = embedding_config(DashscopeFamily::QwenCloud, &env_with_keys());
    assert_eq!(
        config.identity.default_base,
        "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
    );
    assert_eq!(
        config
            .identity
            .env_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-brand")
    );
}

#[rstest]
#[case::dashscope(
    DashscopeFamily::Dashscope,
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::qwencloud(
    DashscopeFamily::QwenCloud,
    "https://dashscope-intl.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::qwen_ai_platform(
    DashscopeFamily::QwenAiPlatform,
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
)]
fn rerank_settings_carry_the_family_url_and_environment_values(
    #[case] family: DashscopeFamily,
    #[case] default_rerank_url: &str,
) {
    let settings = rerank_settings(family, &env_with_keys());
    assert_eq!(settings.default_rerank_url, default_rerank_url);
    assert_eq!(settings.env_value, "https://env.test/reranks");
    let without_env = rerank_settings(family, &no_env());
    assert_eq!(without_env.env_value, "");
}

#[test]
fn rerank_config_carries_the_family_identity_and_settings() {
    let family = DashscopeFamily::QwenAiPlatform;
    let config = rerank_config(family, &env_with_keys());
    assert_eq!(
        config.identity.missing_key_message,
        "Qianwen AI Platform API key is required. Set 'QWEN_AI_PLATFORM_API_KEY' or 'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
    );
    assert_eq!(
        config
            .identity
            .env_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-brand")
    );
    assert_eq!(config.settings, rerank_settings(family, &env_with_keys()));
}

#[test]
fn the_plain_image_identity_takes_the_international_host_and_the_dashscope_key() {
    let identity = image_identity(DashscopeFamily::Dashscope, &env_with_keys());
    assert_eq!(
        identity.default_image_base,
        "https://dashscope-intl.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation"
    );
    assert_eq!(identity.missing_key_message, "DASHSCOPE_API_KEY is not set");
    assert_eq!(
        identity.env_image_base.as_deref(),
        Some("https://env.test/image")
    );
    assert_eq!(
        identity.env_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-dash")
    );
    assert_eq!(
        identity
            .fallback_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        None
    );
}

#[test]
fn image_config_carries_the_family_identity() {
    let config = image_config(DashscopeFamily::QwenAiPlatform, &env_with_keys());
    assert_eq!(
        config.identity.default_image_base,
        "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation"
    );
    assert_eq!(
        config
            .identity
            .env_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-brand")
    );
}

#[rstest]
#[case::dashscope(DashscopeFamily::Dashscope)]
#[case::qwencloud(DashscopeFamily::QwenCloud)]
#[case::qwen_ai_platform(DashscopeFamily::QwenAiPlatform)]
fn a_non_empty_argument_wins_for_every_family(#[case] family: DashscopeFamily) {
    let brand = SecretString::from("sk-brand");
    let dashscope = SecretString::from("sk-dash");
    let key = resolve_family_api_key(family, Some("sk-arg"), Some(&brand), Some(&dashscope));
    assert_eq!(
        key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-arg")
    );
}

#[rstest]
#[case::brand_key_wins(None, Some("sk-brand"), Some("sk-brand"))]
#[case::an_empty_argument_keeps_the_brand(Some(""), Some("sk-brand"), Some("sk-brand"))]
#[case::no_brand_key_falls_through(None, None, Some("sk-dash"))]
#[case::an_empty_brand_key_falls_through(None, Some(""), Some("sk-dash"))]
fn a_qwencloud_key_takes_the_brand_key_before_the_dashscope_key(
    #[case] argument: Option<&str>,
    #[case] brand_value: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let brand = brand_value.map(SecretString::from);
    let dashscope = SecretString::from("sk-dash");
    let key = resolve_family_api_key(
        DashscopeFamily::QwenCloud,
        argument,
        brand.as_ref(),
        Some(&dashscope),
    );
    assert_eq!(key.as_ref().map(ExposeSecret::expose_secret), expected);
}

#[test]
fn a_platform_key_takes_the_brand_key_before_the_dashscope_key() {
    let brand = SecretString::from("sk-brand");
    let dashscope = SecretString::from("sk-dash");
    let key = resolve_family_api_key(
        DashscopeFamily::QwenAiPlatform,
        None,
        Some(&brand),
        Some(&dashscope),
    );
    assert_eq!(
        key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-brand")
    );
}

#[test]
fn the_plain_family_never_reads_the_brand_key() {
    let brand = SecretString::from("sk-brand");
    let dashscope = SecretString::from("sk-dash");
    let key = resolve_family_api_key(
        DashscopeFamily::Dashscope,
        None,
        Some(&brand),
        Some(&dashscope),
    );
    assert_eq!(
        key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-dash")
    );
}

#[rstest]
#[case::dashscope(DashscopeFamily::Dashscope)]
#[case::qwencloud(DashscopeFamily::QwenCloud)]
#[case::qwen_ai_platform(DashscopeFamily::QwenAiPlatform)]
fn no_key_at_all_resolves_to_none(#[case] family: DashscopeFamily) {
    assert!(resolve_family_api_key(family, None, None, None).is_none());
    let empty = SecretString::from("");
    let key = resolve_family_api_key(family, Some(""), Some(&empty), Some(&empty));
    assert_eq!(key.as_ref().map(ExposeSecret::expose_secret), None);
}

#[rstest]
#[case::dashscope(
    DashscopeFamily::Dashscope,
    "Missing API key for DashScope. Set DASHSCOPE_API_KEY environment variable or pass api_key parameter."
)]
#[case::qwencloud(
    DashscopeFamily::QwenCloud,
    "Missing API key for QwenCloud. Set QWENCLOUD_API_KEY or DASHSCOPE_API_KEY environment variable or pass api_key parameter."
)]
#[case::qwen_ai_platform(
    DashscopeFamily::QwenAiPlatform,
    "Missing API key for Qianwen AI Platform. Set QWEN_AI_PLATFORM_API_KEY or DASHSCOPE_API_KEY environment variable or pass api_key parameter."
)]
fn the_missing_key_message_names_each_family(
    #[case] family: DashscopeFamily,
    #[case] expected: &str,
) {
    assert_eq!(missing_family_key_message(family), expected);
}

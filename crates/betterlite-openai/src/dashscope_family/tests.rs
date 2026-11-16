#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    DASHSCOPE_RERANK_URL, DashscopeFamily, FamilyEnv, QWEN_AI_PLATFORM_CHAT_BASE,
    QWEN_AI_PLATFORM_IMAGE_BASE, QWEN_AI_PLATFORM_KEY_MESSAGE, QWEN_AI_PLATFORM_RERANK_URL,
    QWENCLOUD_CHAT_BASE, QWENCLOUD_IMAGE_BASE, QWENCLOUD_KEY_MESSAGE, QWENCLOUD_RERANK_URL,
    chat_policy, embedding_config, embedding_identity, image_config, image_identity,
    missing_family_key_message, rerank_config, rerank_settings, resolve_family_api_key,
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
#[case::dashscope(DashscopeFamily::Dashscope, crate::dashscope_chat::DEFAULT_BASE, false)]
#[case::qwencloud(DashscopeFamily::QwenCloud, QWENCLOUD_CHAT_BASE, true)]
#[case::qwen_ai_platform(DashscopeFamily::QwenAiPlatform, QWEN_AI_PLATFORM_CHAT_BASE, true)]
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

#[test]
fn the_plain_embedding_identity_reads_only_the_dashscope_key() {
    let identity = embedding_identity(DashscopeFamily::Dashscope, &env_with_keys());
    assert_eq!(identity.default_base, crate::dashscope_chat::DEFAULT_BASE);
    assert_eq!(
        identity.missing_key_message,
        crate::dashscope_embedding::DASHSCOPE_MISSING_KEY_MESSAGE
    );
    assert_eq!(identity.env_base.as_deref(), Some("https://env.test/v1"));
    assert_eq!(
        identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-dash")
    );
    assert_eq!(
        identity
            .fallback_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        None
    );
    assert!(identity.empty_key_is_set);
}

#[rstest]
#[case::qwencloud(DashscopeFamily::QwenCloud, QWENCLOUD_CHAT_BASE, QWENCLOUD_KEY_MESSAGE)]
#[case::qwen_ai_platform(
    DashscopeFamily::QwenAiPlatform,
    QWEN_AI_PLATFORM_CHAT_BASE,
    QWEN_AI_PLATFORM_KEY_MESSAGE
)]
fn a_brand_embedding_identity_reads_the_brand_key_before_the_dashscope_key(
    #[case] family: DashscopeFamily,
    #[case] default_base: &str,
    #[case] message: &str,
) {
    let identity = embedding_identity(family, &env_with_keys());
    assert_eq!(identity.default_base, default_base);
    assert_eq!(identity.missing_key_message, message);
    assert_eq!(identity.env_base.as_deref(), Some("https://env.test/v1"));
    assert_eq!(
        identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-brand")
    );
    assert_eq!(
        identity
            .fallback_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-dash")
    );
    assert!(!identity.empty_key_is_set);
}

#[test]
fn embedding_config_carries_the_family_identity() {
    let config = embedding_config(DashscopeFamily::QwenCloud, &env_with_keys());
    assert_eq!(config.identity.default_base, QWENCLOUD_CHAT_BASE);
    assert_eq!(
        config
            .identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-brand")
    );
}

#[rstest]
#[case::dashscope(DashscopeFamily::Dashscope, DASHSCOPE_RERANK_URL)]
#[case::qwencloud(DashscopeFamily::QwenCloud, QWENCLOUD_RERANK_URL)]
#[case::qwen_ai_platform(DashscopeFamily::QwenAiPlatform, QWEN_AI_PLATFORM_RERANK_URL)]
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
        QWEN_AI_PLATFORM_KEY_MESSAGE
    );
    assert_eq!(
        config
            .identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-brand")
    );
    assert_eq!(config.settings, rerank_settings(family, &env_with_keys()));
}

#[test]
fn the_plain_image_identity_takes_the_international_host_and_the_dashscope_key() {
    let identity = image_identity(DashscopeFamily::Dashscope, &env_with_keys());
    assert_eq!(
        identity.default_image_base,
        crate::dashscope_image::DEFAULT_API_BASE
    );
    assert_eq!(
        identity.missing_key_message,
        crate::dashscope_image::MISSING_KEY_MESSAGE
    );
    assert_eq!(
        identity.env_image_base.as_deref(),
        Some("https://env.test/image")
    );
    assert_eq!(
        identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-dash")
    );
    assert_eq!(
        identity
            .fallback_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        None
    );
}

#[rstest]
#[case::qwencloud(
    DashscopeFamily::QwenCloud,
    QWENCLOUD_IMAGE_BASE,
    QWENCLOUD_KEY_MESSAGE
)]
#[case::qwen_ai_platform(
    DashscopeFamily::QwenAiPlatform,
    QWEN_AI_PLATFORM_IMAGE_BASE,
    QWEN_AI_PLATFORM_KEY_MESSAGE
)]
fn a_brand_image_identity_reads_the_brand_key_and_its_own_host(
    #[case] family: DashscopeFamily,
    #[case] default_image_base: &str,
    #[case] message: &str,
) {
    let identity = image_identity(family, &env_with_keys());
    assert_eq!(identity.default_image_base, default_image_base);
    assert_eq!(identity.missing_key_message, message);
    assert_eq!(
        identity.env_image_base.as_deref(),
        Some("https://env.test/image")
    );
    assert_eq!(
        identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-brand")
    );
    assert_eq!(
        identity
            .fallback_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-dash")
    );
}

#[test]
fn image_config_carries_the_family_identity() {
    let config = image_config(DashscopeFamily::QwenAiPlatform, &env_with_keys());
    assert_eq!(
        config.identity.default_image_base,
        QWEN_AI_PLATFORM_IMAGE_BASE
    );
    assert_eq!(
        config
            .identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
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
        key.as_ref().map(|key| key.expose_secret().as_str()),
        Some("sk-arg")
    );
}

#[rstest]
#[case::qwencloud(DashscopeFamily::QwenCloud)]
#[case::qwen_ai_platform(DashscopeFamily::QwenAiPlatform)]
fn a_brand_takes_the_brand_key_before_the_dashscope_key(#[case] family: DashscopeFamily) {
    let brand = SecretString::from("sk-brand");
    let dashscope = SecretString::from("sk-dash");
    let given = resolve_family_api_key(family, None, Some(&brand), Some(&dashscope));
    let empty_argument = resolve_family_api_key(family, Some(""), Some(&brand), Some(&dashscope));
    assert_eq!(
        given.as_ref().map(|key| key.expose_secret().as_str()),
        Some("sk-brand")
    );
    assert_eq!(
        empty_argument
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-brand")
    );
}

#[rstest]
#[case::qwencloud(DashscopeFamily::QwenCloud)]
#[case::qwen_ai_platform(DashscopeFamily::QwenAiPlatform)]
fn a_brand_falls_through_an_absent_or_empty_brand_key(#[case] family: DashscopeFamily) {
    let dashscope = SecretString::from("sk-dash");
    let empty = SecretString::from("");
    let absent = resolve_family_api_key(family, None, None, Some(&dashscope));
    let blank = resolve_family_api_key(family, None, Some(&empty), Some(&dashscope));
    assert_eq!(
        absent.as_ref().map(|key| key.expose_secret().as_str()),
        Some("sk-dash")
    );
    assert_eq!(
        blank.as_ref().map(|key| key.expose_secret().as_str()),
        Some("sk-dash")
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
        key.as_ref().map(|key| key.expose_secret().as_str()),
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
    assert_eq!(key.as_ref().map(|key| key.expose_secret().as_str()), None);
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

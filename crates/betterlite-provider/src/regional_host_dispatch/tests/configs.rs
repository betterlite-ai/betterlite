use std::collections::BTreeMap;

use betterlite_config::sections::regional_host_keys::Settings;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_context::resource_config_selection::responses::ResponsesConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::regional_host_dispatch::configs::{
    RegionalImageConfig, image_config, rerank_config, responses_config,
};

use super::support::secret;

// B8172's plain brand reads DASHSCOPE_API_BASE_IMAGE and DASHSCOPE_API_KEY
#[test]
fn dashscope_family_image_tag_builds_the_brand_config() {
    let mut settings = Settings::default();
    settings.environment.dashscope_image_base = Some(String::from("https://dash-image.example"));
    settings.environment.dashscope_api_key = Some(secret("sk-dashscope"));
    let tag = ImageGenerationConfigTag::DashscopeFamily(String::from("dashscope"));
    let Some(RegionalImageConfig::DashscopeFamily(config)) = image_config(&tag, &settings) else {
        panic!("the dashscope family tag builds B8172's config");
    };
    assert_eq!(config.complete_url(None), "https://dash-image.example");
    let headers = config
        .validate_environment(None, &BTreeMap::new())
        .expect("the environment key resolves");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-dashscope")
    );
}

// the brand's own image base and key win over the DashScope ones
#[test]
fn qwencloud_image_tag_reads_its_own_base_and_key() {
    let mut settings = Settings::default();
    settings.environment.qwencloud_image_base = Some(String::from("https://qwen-image.example"));
    settings.environment.qwencloud_key = Some(secret("sk-qwencloud"));
    settings.environment.dashscope_image_base = Some(String::from("https://dash-image.example"));
    settings.environment.dashscope_api_key = Some(secret("sk-dashscope"));
    let tag = ImageGenerationConfigTag::DashscopeFamily(String::from("qwencloud"));
    let Some(RegionalImageConfig::DashscopeFamily(config)) = image_config(&tag, &settings) else {
        panic!("the qwencloud family tag builds B8172's config");
    };
    assert_eq!(config.complete_url(None), "https://qwen-image.example");
    let headers = config
        .validate_environment(None, &BTreeMap::new())
        .expect("the brand key resolves");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-qwencloud")
    );
}

// the Qianwen AI Platform brand reads QWEN_AI_PLATFORM_API_BASE_IMAGE and QWEN_AI_PLATFORM_API_KEY, not the QwenCloud or DashScope ones
#[test]
fn qwen_ai_platform_image_tag_reads_its_own_base_and_key() {
    let mut settings = Settings::default();
    settings.environment.qwen_ai_platform_image_base =
        Some(String::from("https://qwen-platform-image.example"));
    settings.environment.qwen_ai_platform_key = Some(secret("sk-qwen-platform"));
    settings.environment.qwencloud_image_base = Some(String::from("https://qwen-image.example"));
    settings.environment.qwencloud_key = Some(secret("sk-qwencloud"));
    settings.environment.dashscope_image_base = Some(String::from("https://dash-image.example"));
    settings.environment.dashscope_api_key = Some(secret("sk-dashscope"));
    let tag = ImageGenerationConfigTag::DashscopeFamily(String::from("qwen_ai_platform"));
    let Some(RegionalImageConfig::DashscopeFamily(config)) = image_config(&tag, &settings) else {
        panic!("the qwen_ai_platform family tag builds B8172's config");
    };
    assert_eq!(
        config.complete_url(None),
        "https://qwen-platform-image.example"
    );
    let headers = config
        .validate_environment(None, &BTreeMap::new())
        .expect("the brand key resolves");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-qwen-platform")
    );
}

// B8174's ModelScope config carries the environment values its request functions take
#[test]
fn modelscope_image_tag_builds_the_environment_config() {
    let mut settings = Settings::default();
    settings.environment.modelscope_api_base = Some(String::from("https://modelscope.example/v1"));
    settings.environment.modelscope_api_key = Some(secret("sk-modelscope"));
    let tag = ImageGenerationConfigTag::ModelScope(String::from("Qwen/Qwen-Image"));
    let Some(RegionalImageConfig::ModelScope(config)) = image_config(&tag, &settings) else {
        panic!("the modelscope tag builds B8174's config");
    };
    assert_eq!(
        config.environment_base.as_deref(),
        Some("https://modelscope.example/v1")
    );
    assert_eq!(
        config
            .environment_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from("sk-modelscope"))
    );
}

#[test]
fn other_image_tags_build_no_config() {
    let settings = Settings::default();
    let openai = ImageGenerationConfigTag::OpenAi(String::from("gpt-image-1"));
    assert!(image_config(&openai, &settings).is_none());
    assert!(image_config(&ImageGenerationConfigTag::EdenAi, &settings).is_none());
}

// B8171's config reads the brand's own rerank base and key, and keeps the brand's default URL
#[rstest]
#[case::dashscope(
    ProviderId::Dashscope,
    "https://dash-rerank.example",
    "sk-dashscope",
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::qwencloud(
    ProviderId::Qwencloud,
    "https://qwencloud-rerank.example",
    "sk-qwencloud",
    "https://dashscope-intl.aliyuncs.com/compatible-api/v1/reranks"
)]
#[case::qwen_ai_platform(
    ProviderId::QwenAiPlatform,
    "https://qwen-platform-rerank.example",
    "sk-qwen-platform",
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks"
)]
fn dashscope_rerank_selection_builds_the_rerank_config(
    #[case] provider: ProviderId,
    #[case] expected_env: &str,
    #[case] expected_key: &str,
    #[case] expected_default: &str,
) {
    let mut settings = Settings::default();
    settings.environment.dashscope_rerank_base = Some(String::from("https://dash-rerank.example"));
    settings.environment.qwencloud_rerank_base =
        Some(String::from("https://qwencloud-rerank.example"));
    settings.environment.qwen_ai_platform_rerank_base =
        Some(String::from("https://qwen-platform-rerank.example"));
    settings.environment.dashscope_api_key = Some(secret("sk-dashscope"));
    settings.environment.qwencloud_key = Some(secret("sk-qwencloud"));
    settings.environment.qwen_ai_platform_key = Some(secret("sk-qwen-platform"));
    let config = rerank_config(provider, &settings).expect("the brand has a rerank config");
    assert_eq!(config.rerank_env_value, expected_env);
    assert_eq!(config.default_rerank_url, expected_default);
    assert_eq!(
        config
            .identity
            .env_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some(String::from(expected_key))
    );
}

#[test]
fn other_rerank_providers_build_no_config() {
    let settings = Settings::default();
    assert!(rerank_config(ProviderId::Cohere, &settings).is_none());
}

#[test]
fn volcengine_responses_selection_builds_the_config() {
    let settings = Settings::default();
    assert!(responses_config(&ResponsesConfigTag::VolcEngine, &settings).is_some());
    assert!(responses_config(&ResponsesConfigTag::OpenAi, &settings).is_none());
}

// B8176's chains put the request's key and base before litellm's global ones, and each before its environment variables
#[test]
fn volcengine_responses_config_uses_b8176_key_and_base_chains() {
    let mut settings = Settings::default();
    settings.api_key = Some(secret("sk-global"));
    settings.api_base = Some(String::from("https://global.example"));
    settings.environment.ark_api_key = Some(secret("sk-ark"));
    settings.environment.volcengine_api_key = Some(secret("sk-volcengine"));
    settings.environment.volcengine_api_base = Some(String::from("https://volcengine.example"));
    settings.environment.ark_api_base = Some(String::from("https://ark.example"));
    let config = responses_config(&ResponsesConfigTag::VolcEngine, &settings)
        .expect("the volcengine tag builds B8176's config");
    let request_key = secret("sk-request");
    let request_headers = config
        .validate_environment(&BTreeMap::new(), Some(&request_key))
        .expect("the request key resolves");
    assert_eq!(
        request_headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-request")
    );
    assert_eq!(
        config.complete_url(Some("https://request.example")),
        "https://request.example/api/v3/responses"
    );
    let global_headers = config
        .validate_environment(&BTreeMap::new(), None)
        .expect("the global key resolves");
    assert_eq!(
        global_headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-global")
    );
    assert_eq!(
        config.complete_url(None),
        "https://global.example/api/v3/responses"
    );
}

// with no request or global value the environment answers: ARK_API_KEY before VOLCENGINE_API_KEY, and VOLCENGINE_API_BASE before ARK_API_BASE
#[test]
fn volcengine_responses_config_falls_back_to_the_environment() {
    let mut settings = Settings::default();
    settings.environment.ark_api_key = Some(secret("sk-ark"));
    settings.environment.volcengine_api_key = Some(secret("sk-volcengine"));
    settings.environment.volcengine_api_base = Some(String::from("https://volcengine.example"));
    settings.environment.ark_api_base = Some(String::from("https://ark.example"));
    let config = responses_config(&ResponsesConfigTag::VolcEngine, &settings)
        .expect("the tag builds a config");
    let headers = config
        .validate_environment(&BTreeMap::new(), None)
        .expect("ARK_API_KEY resolves a key");
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-ark")
    );
    assert_eq!(
        config.complete_url(None),
        "https://volcengine.example/api/v3/responses"
    );
    let empty = Settings::default();
    let empty_config = responses_config(&ResponsesConfigTag::VolcEngine, &empty)
        .expect("the tag still builds a config");
    let error = empty_config
        .validate_environment(&BTreeMap::new(), None)
        .expect_err("the missing key is an error");
    assert_eq!(
        error.to_string(),
        "api_key is required for Volcengine authentication"
    );
}

#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::regional_host_keys::{Headers, Settings};

#[derive(Clone, Copy, Debug)]
enum Setting {
    ApiBase,
    ApiKey,
    Headers,
    GigachatKey,
    NlpCloudKey,
    GigachatApiKey,
    GigachatCredentials,
    NlpCloudApiKey,
    NlpCloudApiBase,
    MinimaxApiKey,
    MinimaxApiBase,
    ArkApiKey,
    VolcengineApiKey,
    VolcengineApiBase,
    ArkApiBase,
    DashscopeApiKey,
    DashscopeBase,
    DashscopeRerankBase,
    DashscopeImageBase,
    QwencloudKey,
    QwencloudBase,
    QwencloudRerankBase,
    QwencloudImageBase,
    QwenAiPlatformKey,
    QwenAiPlatformBase,
    QwenAiPlatformRerankBase,
    QwenAiPlatformImageBase,
    ModelscopeApiBase,
    ModelscopeApiKey,
    TencentApiBase,
    TencentApiKey,
}

const ALL: [Setting; 31] = [
    Setting::ApiBase,
    Setting::ApiKey,
    Setting::Headers,
    Setting::GigachatKey,
    Setting::NlpCloudKey,
    Setting::GigachatApiKey,
    Setting::GigachatCredentials,
    Setting::NlpCloudApiKey,
    Setting::NlpCloudApiBase,
    Setting::MinimaxApiKey,
    Setting::MinimaxApiBase,
    Setting::ArkApiKey,
    Setting::VolcengineApiKey,
    Setting::VolcengineApiBase,
    Setting::ArkApiBase,
    Setting::DashscopeApiKey,
    Setting::DashscopeBase,
    Setting::DashscopeRerankBase,
    Setting::DashscopeImageBase,
    Setting::QwencloudKey,
    Setting::QwencloudBase,
    Setting::QwencloudRerankBase,
    Setting::QwencloudImageBase,
    Setting::QwenAiPlatformKey,
    Setting::QwenAiPlatformBase,
    Setting::QwenAiPlatformRerankBase,
    Setting::QwenAiPlatformImageBase,
    Setting::ModelscopeApiBase,
    Setting::ModelscopeApiKey,
    Setting::TencentApiBase,
    Setting::TencentApiKey,
];

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .regional_host_keys)
}

fn header_text(headers: &Headers) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn read(settings: &Settings, setting: Setting) -> Option<String> {
    let secret =
        |value: &Option<SecretString>| value.as_ref().map(|key| key.expose_secret().to_owned());
    let environment = &settings.environment;
    match setting {
        Setting::ApiBase => settings.api_base.clone(),
        Setting::ApiKey => secret(&settings.api_key),
        Setting::Headers => settings.headers.as_ref().map(header_text),
        Setting::GigachatKey => secret(&settings.gigachat_key),
        Setting::NlpCloudKey => secret(&settings.nlp_cloud_key),
        Setting::GigachatApiKey => secret(&environment.gigachat_api_key),
        Setting::GigachatCredentials => secret(&environment.gigachat_credentials),
        Setting::NlpCloudApiKey => secret(&environment.nlp_cloud_api_key),
        Setting::NlpCloudApiBase => environment.nlp_cloud_api_base.clone(),
        Setting::MinimaxApiKey => secret(&environment.minimax_api_key),
        Setting::MinimaxApiBase => environment.minimax_api_base.clone(),
        Setting::ArkApiKey => secret(&environment.ark_api_key),
        Setting::VolcengineApiKey => secret(&environment.volcengine_api_key),
        Setting::VolcengineApiBase => environment.volcengine_api_base.clone(),
        Setting::ArkApiBase => environment.ark_api_base.clone(),
        Setting::DashscopeApiKey => secret(&environment.dashscope_api_key),
        Setting::DashscopeBase => environment.dashscope_base.clone(),
        Setting::DashscopeRerankBase => environment.dashscope_rerank_base.clone(),
        Setting::DashscopeImageBase => environment.dashscope_image_base.clone(),
        Setting::QwencloudKey => secret(&environment.qwencloud_key),
        Setting::QwencloudBase => environment.qwencloud_base.clone(),
        Setting::QwencloudRerankBase => environment.qwencloud_rerank_base.clone(),
        Setting::QwencloudImageBase => environment.qwencloud_image_base.clone(),
        Setting::QwenAiPlatformKey => secret(&environment.qwen_ai_platform_key),
        Setting::QwenAiPlatformBase => environment.qwen_ai_platform_base.clone(),
        Setting::QwenAiPlatformRerankBase => environment.qwen_ai_platform_rerank_base.clone(),
        Setting::QwenAiPlatformImageBase => environment.qwen_ai_platform_image_base.clone(),
        Setting::ModelscopeApiBase => environment.modelscope_api_base.clone(),
        Setting::ModelscopeApiKey => secret(&environment.modelscope_api_key),
        Setting::TencentApiBase => environment.tencent_api_base.clone(),
        Setting::TencentApiKey => secret(&environment.tencent_api_key),
    }
}

#[test]
fn every_setting_is_unset_by_default() {
    let settings = load("", &[]).unwrap();
    let unset = ALL.map(|setting| read(&settings, setting).is_none());
    assert_eq!(unset, [true; 31]);
}

#[rstest]
#[case::api_base(Setting::ApiBase, "litellm_settings:\n  api_base: http://global:4000\n", &[], Some("http://global:4000"))]
#[case::api_key(Setting::ApiKey, "litellm_settings:\n  api_key: sk-global\n", &[], Some("sk-global"))]
#[case::headers(Setting::Headers, "litellm_settings:\n  headers:\n    x-scope: tenant\n", &[], Some("x-scope=tenant"))]
#[case::gigachat_key(Setting::GigachatKey, "litellm_settings:\n  gigachat_key: sk-gigachat\n", &[], Some("sk-gigachat"))]
#[case::nlp_cloud_key(Setting::NlpCloudKey, "litellm_settings:\n  nlp_cloud_key: sk-nlp\n", &[], Some("sk-nlp"))]
#[case::gigachat_api_key(Setting::GigachatApiKey, "", &[("GIGACHAT_API_KEY", "sk-giga-env")], Some("sk-giga-env"))]
#[case::gigachat_credentials(Setting::GigachatCredentials, "", &[("GIGACHAT_CREDENTIALS", "cred")], Some("cred"))]
#[case::nlp_cloud_api_key(Setting::NlpCloudApiKey, "", &[("NLP_CLOUD_API_KEY", "sk-nlp-env")], Some("sk-nlp-env"))]
#[case::nlp_cloud_api_base(Setting::NlpCloudApiBase, "", &[("NLP_CLOUD_API_BASE", "http://nlp")], Some("http://nlp"))]
#[case::minimax_api_key(Setting::MinimaxApiKey, "", &[("MINIMAX_API_KEY", "sk-minimax")], Some("sk-minimax"))]
#[case::minimax_api_base(Setting::MinimaxApiBase, "", &[("MINIMAX_API_BASE", "http://minimax")], Some("http://minimax"))]
#[case::ark_api_key(Setting::ArkApiKey, "", &[("ARK_API_KEY", "sk-ark")], Some("sk-ark"))]
#[case::volcengine_api_key(Setting::VolcengineApiKey, "", &[("VOLCENGINE_API_KEY", "sk-volc")], Some("sk-volc"))]
#[case::volcengine_api_base(Setting::VolcengineApiBase, "", &[("VOLCENGINE_API_BASE", "http://volc")], Some("http://volc"))]
#[case::ark_api_base(Setting::ArkApiBase, "", &[("ARK_API_BASE", "http://ark")], Some("http://ark"))]
#[case::dashscope_api_key(Setting::DashscopeApiKey, "", &[("DASHSCOPE_API_KEY", "sk-dash")], Some("sk-dash"))]
#[case::dashscope_base(Setting::DashscopeBase, "", &[("DASHSCOPE_API_BASE", "http://dash")], Some("http://dash"))]
#[case::dashscope_rerank_base(Setting::DashscopeRerankBase, "", &[("DASHSCOPE_API_BASE_RERANK", "http://dash-rerank")], Some("http://dash-rerank"))]
#[case::dashscope_image_base(Setting::DashscopeImageBase, "", &[("DASHSCOPE_API_BASE_IMAGE", "http://dash-image")], Some("http://dash-image"))]
#[case::qwencloud_key(Setting::QwencloudKey, "", &[("QWENCLOUD_API_KEY", "sk-qwen")], Some("sk-qwen"))]
#[case::qwencloud_base(Setting::QwencloudBase, "", &[("QWENCLOUD_API_BASE", "http://qwen")], Some("http://qwen"))]
#[case::qwencloud_rerank_base(Setting::QwencloudRerankBase, "", &[("QWENCLOUD_API_BASE_RERANK", "http://qwen-rerank")], Some("http://qwen-rerank"))]
#[case::qwencloud_image_base(Setting::QwencloudImageBase, "", &[("QWENCLOUD_API_BASE_IMAGE", "http://qwen-image")], Some("http://qwen-image"))]
#[case::qwen_ai_platform_key(Setting::QwenAiPlatformKey, "", &[("QWEN_AI_PLATFORM_API_KEY", "sk-qwen-ai")], Some("sk-qwen-ai"))]
#[case::qwen_ai_platform_base(Setting::QwenAiPlatformBase, "", &[("QWEN_AI_PLATFORM_API_BASE", "http://qwen-ai")], Some("http://qwen-ai"))]
#[case::qwen_ai_platform_rerank_base(Setting::QwenAiPlatformRerankBase, "", &[("QWEN_AI_PLATFORM_API_BASE_RERANK", "http://qwen-ai-rerank")], Some("http://qwen-ai-rerank"))]
#[case::qwen_ai_platform_image_base(Setting::QwenAiPlatformImageBase, "", &[("QWEN_AI_PLATFORM_API_BASE_IMAGE", "http://qwen-ai-image")], Some("http://qwen-ai-image"))]
#[case::modelscope_api_base(Setting::ModelscopeApiBase, "", &[("MODELSCOPE_API_BASE", "http://scope")], Some("http://scope"))]
#[case::modelscope_api_key(Setting::ModelscopeApiKey, "", &[("MODELSCOPE_API_KEY", "sk-scope")], Some("sk-scope"))]
#[case::tencent_api_base(Setting::TencentApiBase, "", &[("TENCENT_API_BASE", "http://tencent")], Some("http://tencent"))]
#[case::tencent_api_key(Setting::TencentApiKey, "", &[("TENCENT_API_KEY", "sk-tencent")], Some("sk-tencent"))]
#[case::litellm_api_base_resolves_an_environment_reference(Setting::ApiBase, "litellm_settings:\n  api_base: os.environ/GLOBAL_BASE\n", &[("GLOBAL_BASE", "http://from-env:4000")], Some("http://from-env:4000"))]
#[case::litellm_api_key_resolves_an_environment_reference(Setting::ApiKey, "litellm_settings:\n  api_key: os.environ/GLOBAL_KEY\n", &[("GLOBAL_KEY", "sk-from-env")], Some("sk-from-env"))]
#[case::headers_resolve_an_environment_reference(Setting::Headers, "litellm_settings:\n  headers:\n    Authorization: os.environ/HEADER_KEY\n", &[("HEADER_KEY", "Bearer sk-header")], Some("Authorization=Bearer sk-header"))]
#[case::gigachat_key_resolves_an_environment_reference(Setting::GigachatKey, "litellm_settings:\n  gigachat_key: os.environ/GIGA_GLOBAL\n", &[("GIGA_GLOBAL", "sk-giga-env")], Some("sk-giga-env"))]
#[case::nlp_cloud_key_resolves_an_environment_reference(Setting::NlpCloudKey, "litellm_settings:\n  nlp_cloud_key: os.environ/NLP_GLOBAL\n", &[("NLP_GLOBAL", "sk-nlp-env")], Some("sk-nlp-env"))]
fn loads_each_listed_setting(
    #[case] setting: Setting,
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        read(&load(source, environment).unwrap(), setting),
        expected.map(str::to_owned)
    );
}

#[rstest]
#[case::api_base(Setting::ApiBase, "litellm_settings:\n  api_base: \"\"\n", &[])]
#[case::api_key(Setting::ApiKey, "litellm_settings:\n  api_key: \"\"\n", &[])]
#[case::headers(Setting::Headers, "litellm_settings:\n  headers: {}\n", &[])]
#[case::gigachat_key(Setting::GigachatKey, "litellm_settings:\n  gigachat_key: \"\"\n", &[])]
#[case::nlp_cloud_key(Setting::NlpCloudKey, "litellm_settings:\n  nlp_cloud_key: \"\"\n", &[])]
#[case::gigachat_api_key(Setting::GigachatApiKey, "", &[("GIGACHAT_API_KEY", "")])]
#[case::gigachat_credentials(Setting::GigachatCredentials, "", &[("GIGACHAT_CREDENTIALS", "")])]
#[case::nlp_cloud_api_key(Setting::NlpCloudApiKey, "", &[("NLP_CLOUD_API_KEY", "")])]
#[case::nlp_cloud_api_base(Setting::NlpCloudApiBase, "", &[("NLP_CLOUD_API_BASE", "")])]
#[case::minimax_api_key(Setting::MinimaxApiKey, "", &[("MINIMAX_API_KEY", "")])]
#[case::minimax_api_base(Setting::MinimaxApiBase, "", &[("MINIMAX_API_BASE", "")])]
#[case::ark_api_key(Setting::ArkApiKey, "", &[("ARK_API_KEY", "")])]
#[case::volcengine_api_key(Setting::VolcengineApiKey, "", &[("VOLCENGINE_API_KEY", "")])]
#[case::volcengine_api_base(Setting::VolcengineApiBase, "", &[("VOLCENGINE_API_BASE", "")])]
#[case::ark_api_base(Setting::ArkApiBase, "", &[("ARK_API_BASE", "")])]
#[case::dashscope_api_key(Setting::DashscopeApiKey, "", &[("DASHSCOPE_API_KEY", "")])]
#[case::dashscope_base(Setting::DashscopeBase, "", &[("DASHSCOPE_API_BASE", "")])]
#[case::dashscope_rerank_base(Setting::DashscopeRerankBase, "", &[("DASHSCOPE_API_BASE_RERANK", "")])]
#[case::dashscope_image_base(Setting::DashscopeImageBase, "", &[("DASHSCOPE_API_BASE_IMAGE", "")])]
#[case::qwencloud_key(Setting::QwencloudKey, "", &[("QWENCLOUD_API_KEY", "")])]
#[case::qwencloud_base(Setting::QwencloudBase, "", &[("QWENCLOUD_API_BASE", "")])]
#[case::qwencloud_rerank_base(Setting::QwencloudRerankBase, "", &[("QWENCLOUD_API_BASE_RERANK", "")])]
#[case::qwencloud_image_base(Setting::QwencloudImageBase, "", &[("QWENCLOUD_API_BASE_IMAGE", "")])]
#[case::qwen_ai_platform_key(Setting::QwenAiPlatformKey, "", &[("QWEN_AI_PLATFORM_API_KEY", "")])]
#[case::qwen_ai_platform_base(Setting::QwenAiPlatformBase, "", &[("QWEN_AI_PLATFORM_API_BASE", "")])]
#[case::qwen_ai_platform_rerank_base(Setting::QwenAiPlatformRerankBase, "", &[("QWEN_AI_PLATFORM_API_BASE_RERANK", "")])]
#[case::qwen_ai_platform_image_base(Setting::QwenAiPlatformImageBase, "", &[("QWEN_AI_PLATFORM_API_BASE_IMAGE", "")])]
#[case::modelscope_api_base(Setting::ModelscopeApiBase, "", &[("MODELSCOPE_API_BASE", "")])]
#[case::modelscope_api_key(Setting::ModelscopeApiKey, "", &[("MODELSCOPE_API_KEY", "")])]
#[case::tencent_api_base(Setting::TencentApiBase, "", &[("TENCENT_API_BASE", "")])]
#[case::tencent_api_key(Setting::TencentApiKey, "", &[("TENCENT_API_KEY", "")])]
fn an_empty_value_counts_as_unset(
    #[case] setting: Setting,
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
) {
    assert_eq!(read(&load(source, environment).unwrap(), setting), None);
}

#[test]
fn a_missing_environment_reference_is_an_error() {
    let error = load("litellm_settings:\n  api_key: os.environ/ABSENT_KEY\n", &[]).unwrap_err();
    assert!(error.to_string().contains("ABSENT_KEY"), "{error}");
}

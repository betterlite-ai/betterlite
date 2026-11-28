use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_context::resource_config_selection::responses::ResponsesConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::dashscope_family::{self, DashscopeFamily, FamilyEnv};
use betterlite_openai::dashscope_image::DashscopeImageConfig;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::volcengine_responses::error::VolcengineResponsesError;
use betterlite_openai::volcengine_responses::request::{
    BaseSources, KeySources, get_complete_url as volcengine_url,
    validate_environment as volcengine_headers,
};
use secrecy::SecretString;

use crate::dashscope_rerank::DashscopeRerankConfig;

const FAMILIES: [(ProviderId, DashscopeFamily); 3] = [
    (ProviderId::Dashscope, DashscopeFamily::Dashscope),
    (ProviderId::Qwencloud, DashscopeFamily::QwenCloud),
    (ProviderId::QwenAiPlatform, DashscopeFamily::QwenAiPlatform),
];

// B2230 and B5360 name one of the three brands by the provider that was called
fn family(provider: ProviderId) -> Option<DashscopeFamily> {
    FAMILIES
        .iter()
        .find_map(|row| (row.0 == provider).then_some(row.1))
}

// the environment the brand's B8171 and B8172 configs read, in the fields those parts take
#[rustfmt::skip]
fn family_env(brand: DashscopeFamily, settings: &SectionSettings) -> FamilyEnv {
    let environment = &settings.environment;
    let (base, brand_key, rerank_base, image_base) = match brand {
        DashscopeFamily::Dashscope => (&environment.dashscope_base, None, &environment.dashscope_rerank_base, &environment.dashscope_image_base),
        DashscopeFamily::QwenCloud => (&environment.qwencloud_base, environment.qwencloud_key.as_ref(), &environment.qwencloud_rerank_base, &environment.qwencloud_image_base),
        DashscopeFamily::QwenAiPlatform => (&environment.qwen_ai_platform_base, environment.qwen_ai_platform_key.as_ref(), &environment.qwen_ai_platform_rerank_base, &environment.qwen_ai_platform_image_base),
    };
    FamilyEnv {
        base: base.clone(),
        brand_key: brand_key.cloned(),
        fallback_key: environment.dashscope_api_key.clone(),
        rerank_base: rerank_base.clone(),
        image_base: image_base.clone(),
    }
}

// the image config B5360's tag names: B8172's for the three brands, B8174's for ModelScope
#[derive(Debug, Clone)]
pub enum RegionalImageConfig {
    DashscopeFamily(DashscopeImageConfig),
    ModelScope(ModelscopeImageConfig),
}

// B8174's ModelScope image config: the two environment values its request functions take
#[derive(Debug, Clone, Default)]
pub struct ModelscopeImageConfig {
    pub environment_base: Option<String>,
    pub environment_key: Option<SecretString>,
}

#[rustfmt::skip]
#[must_use]
pub fn image_config(tag: &ImageGenerationConfigTag, settings: &SectionSettings) -> Option<RegionalImageConfig> {
    match tag {
        ImageGenerationConfigTag::DashscopeFamily(name) => {
            let brand = family(name.parse::<ProviderId>().ok()?)?;
            Some(RegionalImageConfig::DashscopeFamily(dashscope_family::image_config(brand, &family_env(brand, settings))))
        }
        ImageGenerationConfigTag::ModelScope(_model) => Some(RegionalImageConfig::ModelScope(ModelscopeImageConfig {
            environment_base: settings.environment.modelscope_api_base.clone(),
            environment_key: settings.environment.modelscope_api_key.clone(),
        })),
        ImageGenerationConfigTag::OpenAi(_) | ImageGenerationConfigTag::Azure(_) | ImageGenerationConfigTag::AzureAi(_) | ImageGenerationConfigTag::Xinference(_) | ImageGenerationConfigTag::Recraft(_) | ImageGenerationConfigTag::Aiml(_) | ImageGenerationConfigTag::CometApi(_) | ImageGenerationConfigTag::Gemini(_) | ImageGenerationConfigTag::LiteLlmProxy | ImageGenerationConfigTag::FalAi(_) | ImageGenerationConfigTag::Stability(_) | ImageGenerationConfigTag::RunwayMl(_) | ImageGenerationConfigTag::BlackForestLabs(_) | ImageGenerationConfigTag::VertexAi(_) | ImageGenerationConfigTag::OpenRouter(_) | ImageGenerationConfigTag::EdenAi => None,
    }
}

// B8171's rerank config for the three brands, from the provider the rerank selection named
#[must_use]
pub fn rerank_config(
    provider: ProviderId,
    settings: &SectionSettings,
) -> Option<DashscopeRerankConfig> {
    let brand = family(provider)?;
    let config = dashscope_family::rerank_config(brand, &family_env(brand, settings));
    Some(DashscopeRerankConfig {
        identity: config.identity,
        rerank_env_value: config.settings.env_value,
        default_rerank_url: config.settings.default_rerank_url,
    })
}

// B8176's config for the Responses selection: Volcengine's tag is the only one it answers
pub struct VolcengineResponsesConfig<'settings> {
    settings: &'settings SectionSettings,
}

#[rustfmt::skip]
#[must_use]
pub const fn responses_config<'settings>(tag: &ResponsesConfigTag, settings: &'settings SectionSettings) -> Option<VolcengineResponsesConfig<'settings>> {
    match tag {
        ResponsesConfigTag::VolcEngine => Some(VolcengineResponsesConfig { settings }),
        ResponsesConfigTag::OpenAi | ResponsesConfigTag::AzureOSeries | ResponsesConfigTag::Azure | ResponsesConfigTag::AzureAi | ResponsesConfigTag::Xai | ResponsesConfigTag::Sail | ResponsesConfigTag::GithubCopilot | ResponsesConfigTag::ChatGpt | ResponsesConfigTag::LiteLlmProxy | ResponsesConfigTag::Manus | ResponsesConfigTag::Perplexity | ResponsesConfigTag::Databricks | ResponsesConfigTag::OpenRouter | ResponsesConfigTag::HostedVllm | ResponsesConfigTag::FireworksAi | ResponsesConfigTag::EdenAi | ResponsesConfigTag::Bedrock(_) | ResponsesConfigTag::BedrockMantle(_) => None,
    }
}

impl VolcengineResponsesConfig<'_> {
    // B8176's key chain over the regional settings: the request's, the global, ARK_API_KEY, then VOLCENGINE_API_KEY
    pub fn validate_environment(
        &self,
        headers: &Headers,
        request_key: Option<&SecretString>,
    ) -> Result<Headers, VolcengineResponsesError> {
        let environment = &self.settings.environment;
        volcengine_headers(
            headers,
            KeySources {
                litellm_params: request_key,
                global: self.settings.api_key.as_ref(),
                ark: environment.ark_api_key.as_ref(),
                volcengine: environment.volcengine_api_key.as_ref(),
            },
        )
    }

    // B8176's base chain: the request's, the global, VOLCENGINE_API_BASE, then ARK_API_BASE
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>) -> String {
        let environment = &self.settings.environment;
        volcengine_url(BaseSources {
            api_base,
            global: self.settings.api_base.as_deref(),
            volcengine: environment.volcengine_api_base.as_deref(),
            ark: environment.ark_api_base.as_deref(),
        })
    }
}

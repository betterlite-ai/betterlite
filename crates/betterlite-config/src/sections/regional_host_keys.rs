use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

mod tests;

pub type Headers = BTreeMap<String, String>;

// main.py's GigaChat, MiniMax, NLP Cloud, DashScope family, Volcengine and ModelScope arms: the SDK globals through litellm, the rest through get_secret_str
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    #[serde(default)]
    headers: Headers,
    gigachat_key: Option<SecretString>,
    nlp_cloud_key: Option<SecretString>,
}

// The provider variables each arm falls back to; an empty one counts as unset
#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub gigachat_api_key: Option<SecretString>,
    pub gigachat_credentials: Option<SecretString>,
    pub nlp_cloud_api_key: Option<SecretString>,
    pub nlp_cloud_api_base: Option<String>,
    pub minimax_api_key: Option<SecretString>,
    pub minimax_api_base: Option<String>,
    pub ark_api_key: Option<SecretString>,
    pub volcengine_api_key: Option<SecretString>,
    pub volcengine_api_base: Option<String>,
    pub ark_api_base: Option<String>,
    pub dashscope_api_key: Option<SecretString>,
    pub dashscope_base: Option<String>,
    pub dashscope_rerank_base: Option<String>,
    pub dashscope_image_base: Option<String>,
    pub qwencloud_key: Option<SecretString>,
    pub qwencloud_base: Option<String>,
    pub qwencloud_rerank_base: Option<String>,
    pub qwencloud_image_base: Option<String>,
    pub qwen_ai_platform_key: Option<SecretString>,
    pub qwen_ai_platform_base: Option<String>,
    pub qwen_ai_platform_rerank_base: Option<String>,
    pub qwen_ai_platform_image_base: Option<String>,
    pub modelscope_api_base: Option<String>,
    pub modelscope_api_key: Option<SecretString>,
    pub tencent_api_base: Option<String>,
    pub tencent_api_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub gigachat_key: Option<SecretString>,
    pub nlp_cloud_key: Option<SecretString>,
    pub environment: Environment,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // get_secret_str reads the provider variables; an empty one counts as unset
        let read = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let mut headers = Headers::new();
        for (name, value) in self.litellm_settings.headers {
            let resolved = resolve_string(value, "litellm_settings.headers", lookup)?;
            headers.insert(name, resolved);
        }
        Ok(Settings {
            api_base: resolve_optional(self.litellm_settings.api_base, lookup)?,
            api_key: resolve_secret(
                self.litellm_settings.api_key,
                "litellm_settings.api_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            headers: (!headers.is_empty()).then_some(headers),
            gigachat_key: resolve_secret(
                self.litellm_settings.gigachat_key,
                "litellm_settings.gigachat_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            nlp_cloud_key: resolve_secret(
                self.litellm_settings.nlp_cloud_key,
                "litellm_settings.nlp_cloud_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            environment: Environment {
                gigachat_api_key: read("GIGACHAT_API_KEY").map(SecretString::from),
                gigachat_credentials: read("GIGACHAT_CREDENTIALS").map(SecretString::from),
                nlp_cloud_api_key: read("NLP_CLOUD_API_KEY").map(SecretString::from),
                nlp_cloud_api_base: read("NLP_CLOUD_API_BASE"),
                minimax_api_key: read("MINIMAX_API_KEY").map(SecretString::from),
                minimax_api_base: read("MINIMAX_API_BASE"),
                ark_api_key: read("ARK_API_KEY").map(SecretString::from),
                volcengine_api_key: read("VOLCENGINE_API_KEY").map(SecretString::from),
                volcengine_api_base: read("VOLCENGINE_API_BASE"),
                ark_api_base: read("ARK_API_BASE"),
                dashscope_api_key: read("DASHSCOPE_API_KEY").map(SecretString::from),
                dashscope_base: read("DASHSCOPE_API_BASE"),
                dashscope_rerank_base: read("DASHSCOPE_API_BASE_RERANK"),
                dashscope_image_base: read("DASHSCOPE_API_BASE_IMAGE"),
                qwencloud_key: read("QWENCLOUD_API_KEY").map(SecretString::from),
                qwencloud_base: read("QWENCLOUD_API_BASE"),
                qwencloud_rerank_base: read("QWENCLOUD_API_BASE_RERANK"),
                qwencloud_image_base: read("QWENCLOUD_API_BASE_IMAGE"),
                qwen_ai_platform_key: read("QWEN_AI_PLATFORM_API_KEY").map(SecretString::from),
                qwen_ai_platform_base: read("QWEN_AI_PLATFORM_API_BASE"),
                qwen_ai_platform_rerank_base: read("QWEN_AI_PLATFORM_API_BASE_RERANK"),
                qwen_ai_platform_image_base: read("QWEN_AI_PLATFORM_API_BASE_IMAGE"),
                modelscope_api_base: read("MODELSCOPE_API_BASE"),
                modelscope_api_key: read("MODELSCOPE_API_KEY").map(SecretString::from),
                tencent_api_base: read("TENCENT_API_BASE"),
                tencent_api_key: read("TENCENT_API_KEY").map(SecretString::from),
            },
        })
    }
}

// litellm_settings.api_base resolves an os.environ reference like a header value does
fn resolve_optional<L>(value: Option<String>, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        Some(text) => Ok(
            Some(resolve_string(text, "litellm_settings.api_base", lookup)?)
                .filter(|value| !value.is_empty()),
        ),
        None => Ok(None),
    }
}

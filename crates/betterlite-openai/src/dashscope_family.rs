mod tests;

use secrecy::{ExposeSecret as _, SecretString};

use crate::dashscope_chat::DEFAULT_BASE as CHAT_DEFAULT_BASE;
use crate::dashscope_chat::{ChatBasePolicy, DashscopeChatConfig};
use crate::dashscope_embedding::DASHSCOPE_MISSING_KEY_MESSAGE;
use crate::dashscope_embedding::DashscopeEmbeddingConfig;
use crate::dashscope_embedding::identity::FamilyIdentity;
use crate::dashscope_image::identity::ImageIdentity;
use crate::dashscope_image::{DEFAULT_API_BASE, DashscopeImageConfig, MISSING_KEY_MESSAGE};

const DASHSCOPE_RERANK_URL: &str = "https://dashscope.aliyuncs.com/compatible-api/v1/reranks";
const QWENCLOUD_CHAT_BASE: &str = "https://dashscope-intl.aliyuncs.com/compatible-mode/v1";
const QWENCLOUD_RERANK_URL: &str = "https://dashscope-intl.aliyuncs.com/compatible-api/v1/reranks";
const QWENCLOUD_IMAGE_BASE: &str =
    "https://dashscope-intl.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";
const QWENCLOUD_KEY_MESSAGE: &str = concat!(
    "QwenCloud API key is required. Set 'QWENCLOUD_API_KEY' or ",
    "'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
);
const QWEN_AI_PLATFORM_CHAT_BASE: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
const QWEN_AI_PLATFORM_RERANK_URL: &str =
    "https://dashscope.aliyuncs.com/compatible-api/v1/reranks";
const QWEN_AI_PLATFORM_IMAGE_BASE: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";
const QWEN_AI_PLATFORM_KEY_MESSAGE: &str = concat!(
    "Qianwen AI Platform API key is required. Set 'QWEN_AI_PLATFORM_API_KEY' or ",
    "'DASHSCOPE_API_KEY' env var or pass api_key explicitly."
);
const DASHSCOPE_FAMILY_MESSAGE: &str = concat!(
    "Missing API key for DashScope. Set DASHSCOPE_API_KEY ",
    "environment variable or pass api_key parameter."
);
const QWENCLOUD_FAMILY_MESSAGE: &str = concat!(
    "Missing API key for QwenCloud. Set QWENCLOUD_API_KEY or ",
    "DASHSCOPE_API_KEY environment variable or pass api_key parameter."
);
const QWEN_PLATFORM_FAMILY_MESSAGE: &str = concat!(
    "Missing API key for Qianwen AI Platform. Set ",
    "QWEN_AI_PLATFORM_API_KEY or DASHSCOPE_API_KEY environment variable or pass api_key parameter."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashscopeFamily {
    Dashscope,
    QwenCloud,
    QwenAiPlatform,
}

// the environment values the caller resolved for one family; a brand reads its own key before the DashScope one
#[derive(Debug, Clone)]
pub struct FamilyEnv {
    pub base: Option<String>,
    pub brand_key: Option<SecretString>,
    pub fallback_key: Option<SecretString>,
    pub rerank_base: Option<String>,
    pub image_base: Option<String>,
}

// the two values B8171's DashscopeRerankConfig builds its URL from
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankSettings {
    pub env_value: String,
    pub default_rerank_url: String,
}

// B8171's rerank config inputs, split out because its provider type lives in the provider crate
#[derive(Debug, Clone)]
pub struct RerankConfig {
    pub identity: FamilyIdentity,
    pub settings: RerankSettings,
}

struct FamilyDefaults {
    chat_base: &'static str,
    rerank_url: &'static str,
    image_base: &'static str,
    embedding_missing_key_message: &'static str,
    image_missing_key_message: &'static str,
    env_in_complete_url: bool,
}

const fn defaults(family: DashscopeFamily) -> FamilyDefaults {
    match family {
        DashscopeFamily::Dashscope => FamilyDefaults {
            chat_base: CHAT_DEFAULT_BASE,
            rerank_url: DASHSCOPE_RERANK_URL,
            image_base: DEFAULT_API_BASE,
            embedding_missing_key_message: DASHSCOPE_MISSING_KEY_MESSAGE,
            image_missing_key_message: MISSING_KEY_MESSAGE,
            env_in_complete_url: false,
        },
        DashscopeFamily::QwenCloud => FamilyDefaults {
            chat_base: QWENCLOUD_CHAT_BASE,
            rerank_url: QWENCLOUD_RERANK_URL,
            image_base: QWENCLOUD_IMAGE_BASE,
            embedding_missing_key_message: QWENCLOUD_KEY_MESSAGE,
            image_missing_key_message: QWENCLOUD_KEY_MESSAGE,
            env_in_complete_url: true,
        },
        DashscopeFamily::QwenAiPlatform => FamilyDefaults {
            chat_base: QWEN_AI_PLATFORM_CHAT_BASE,
            rerank_url: QWEN_AI_PLATFORM_RERANK_URL,
            image_base: QWEN_AI_PLATFORM_IMAGE_BASE,
            embedding_missing_key_message: QWEN_AI_PLATFORM_KEY_MESSAGE,
            image_missing_key_message: QWEN_AI_PLATFORM_KEY_MESSAGE,
            env_in_complete_url: true,
        },
    }
}

// a brand's chat URL reads its environment base, the plain family's does not
#[must_use]
pub fn chat_policy(family: DashscopeFamily, env: &FamilyEnv) -> ChatBasePolicy {
    let defaults = defaults(family);
    ChatBasePolicy {
        default_base: defaults.chat_base.to_owned(),
        env_base: env.base.clone(),
        env_in_complete_url: defaults.env_in_complete_url,
    }
}

// the chat config a family hands B8170's chat path: the policy above plus the key the environment resolved
#[must_use]
pub fn chat_config(family: DashscopeFamily, env: &FamilyEnv) -> DashscopeChatConfig {
    DashscopeChatConfig {
        policy: chat_policy(family, env),
        env_api_key: resolve_family_api_key(
            family,
            None,
            env.brand_key.as_ref(),
            env.fallback_key.as_ref(),
        ),
    }
}

// a brand reads its own key before the DashScope one; the plain family has only the DashScope key
fn family_keys(
    family: DashscopeFamily,
    env: &FamilyEnv,
) -> (Option<SecretString>, Option<SecretString>) {
    match family {
        DashscopeFamily::Dashscope => (env.fallback_key.clone(), None),
        DashscopeFamily::QwenCloud | DashscopeFamily::QwenAiPlatform => {
            (env.brand_key.clone(), env.fallback_key.clone())
        }
    }
}

#[must_use]
pub fn embedding_identity(family: DashscopeFamily, env: &FamilyEnv) -> FamilyIdentity {
    let defaults = defaults(family);
    let (env_key, fallback_key) = family_keys(family, env);
    FamilyIdentity {
        missing_key_message: defaults.embedding_missing_key_message.to_owned(),
        default_base: defaults.chat_base.to_owned(),
        env_base: env.base.clone(),
        env_key,
        fallback_key,
        empty_key_is_set: family == DashscopeFamily::Dashscope,
    }
}

#[must_use]
pub fn embedding_config(family: DashscopeFamily, env: &FamilyEnv) -> DashscopeEmbeddingConfig {
    DashscopeEmbeddingConfig {
        identity: embedding_identity(family, env),
    }
}

#[must_use]
pub fn rerank_settings(family: DashscopeFamily, env: &FamilyEnv) -> RerankSettings {
    RerankSettings {
        env_value: env.rerank_base.clone().unwrap_or_default(),
        default_rerank_url: defaults(family).rerank_url.to_owned(),
    }
}

#[must_use]
pub fn rerank_config(family: DashscopeFamily, env: &FamilyEnv) -> RerankConfig {
    RerankConfig {
        identity: embedding_identity(family, env),
        settings: rerank_settings(family, env),
    }
}

#[must_use]
pub fn image_identity(family: DashscopeFamily, env: &FamilyEnv) -> ImageIdentity {
    let defaults = defaults(family);
    let (env_key, fallback_key) = family_keys(family, env);
    ImageIdentity {
        missing_key_message: defaults.image_missing_key_message.to_owned(),
        env_key,
        fallback_key,
        env_image_base: env.image_base.clone(),
        default_image_base: defaults.image_base.to_owned(),
    }
}

#[must_use]
pub fn image_config(family: DashscopeFamily, env: &FamilyEnv) -> DashscopeImageConfig {
    DashscopeImageConfig {
        identity: image_identity(family, env),
    }
}

// the source resolves api_key, then the brand's own key, then DASHSCOPE_API_KEY, skipping empty values
#[must_use]
pub fn resolve_family_api_key(
    family: DashscopeFamily,
    api_key: Option<&str>,
    brand_key: Option<&SecretString>,
    dashscope_key: Option<&SecretString>,
) -> Option<SecretString> {
    let from_argument = api_key
        .filter(|key| !key.is_empty())
        .map(SecretString::from);
    let from_brand = match family {
        DashscopeFamily::Dashscope => None,
        DashscopeFamily::QwenCloud | DashscopeFamily::QwenAiPlatform => brand_key
            .filter(|key| !key.expose_secret().is_empty())
            .cloned(),
    };
    let from_dashscope = dashscope_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned();
    from_argument.or(from_brand).or(from_dashscope)
}

#[must_use]
pub const fn missing_family_key_message(family: DashscopeFamily) -> &'static str {
    match family {
        DashscopeFamily::Dashscope => DASHSCOPE_FAMILY_MESSAGE,
        DashscopeFamily::QwenCloud => QWENCLOUD_FAMILY_MESSAGE,
        DashscopeFamily::QwenAiPlatform => QWEN_PLATFORM_FAMILY_MESSAGE,
    }
}

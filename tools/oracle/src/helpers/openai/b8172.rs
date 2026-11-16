use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_openai::dashscope_chat::DashscopeChatConfig;
use betterlite_openai::dashscope_family::{
    DashscopeFamily, FamilyEnv, RerankConfig, chat_policy, embedding_config, image_config,
    missing_family_key_message, rerank_config, resolve_family_api_key,
};
use betterlite_openai::dashscope_image::error::DashscopeImageError;
use betterlite_openai::dashscope_image::identity::ImageIdentity;
use betterlite_openai::dashscope_image::{
    DEFAULT_API_BASE, DashscopeImageConfig, MISSING_KEY_MESSAGE, decode_dashscope_image_reply,
    map_openai_params, transform_image_generation_request,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_provider::api::rerank_contracts::BaseRerankConfig as _;
use betterlite_provider::dashscope_rerank::DashscopeRerankConfig;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

const RERANK_MODEL: &str = "qwen3-rerank";

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ImageParamsAndSize(ImageParamsAndSizeInput),
    ImageUrlAndKey(ImageUrlAndKeyInput),
    ImageRequest(ImageRequestInput),
    ImageReply(ImageReplyInput),
    FamilySettings(FamilySettingsInput),
    FamilyKeyAndMessage(FamilyKeyAndMessageInput),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageParamsAndSizeInput {
    non_default_params: ImageParams,
    optional_params: ImageParams,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageUrlAndKeyInput {
    api_base: Option<String>,
    api_key: Option<String>,
    env_key: Option<String>,
    env_image_base: Option<String>,
    #[serde(default)]
    caller_headers: Headers,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageRequestInput {
    model: String,
    prompt: String,
    optional_params: ImageParams,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImageReplyInput {
    body: String,
    status: u16,
    #[serde(default)]
    headers: Headers,
}

// one family's resolved environment values, as the caller hands them to the family functions
#[derive(Debug, Deserialize)]
pub(crate) struct FamilyStep {
    base: Option<String>,
    brand_key: Option<String>,
    fallback_key: Option<String>,
    rerank_base: Option<String>,
    image_base: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FamilySettingsInput {
    api_base: Option<String>,
    api_key: Option<String>,
    dashscope: FamilyStep,
    qwencloud: FamilyStep,
    qwen_ai_platform: FamilyStep,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FamilyKeyAndMessageInput {
    api_key: Option<String>,
    dashscope_key: Option<String>,
    qwencloud_key: Option<String>,
    qwen_ai_platform_key: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ImageParamsAndSize(inner) => image_params_and_size(inner),
            Input::ImageUrlAndKey(inner) => image_url_and_key(inner),
            Input::ImageRequest(inner) => image_request(inner),
            Input::ImageReply(inner) => image_reply(inner),
            Input::FamilySettings(inner) => family_settings(inner),
            Input::FamilyKeyAndMessage(inner) => family_key_and_message(inner),
        }
    }
}

fn image_params_and_size(input: &ImageParamsAndSizeInput) -> HelperResult {
    let mapped = map_openai_params(&input.non_default_params, &input.optional_params)
        .map_err(image_failure)?;
    let params = serde_json::to_value(mapped)?;
    Ok(json!({"params": params}))
}

fn image_url_and_key(input: &ImageUrlAndKeyInput) -> HelperResult {
    let config = DashscopeImageConfig {
        identity: ImageIdentity {
            missing_key_message: MISSING_KEY_MESSAGE.to_owned(),
            env_key: input.env_key.as_deref().map(SecretString::from),
            fallback_key: None,
            env_image_base: input.env_image_base.clone(),
            default_image_base: DEFAULT_API_BASE.to_owned(),
        },
    };
    let resolved = config
        .validate_environment(input.api_key.as_deref(), &input.caller_headers)
        .map_err(image_failure)?;
    let url = config.complete_url(input.api_base.as_deref());
    let headers = serde_json::to_value(resolved)?;
    Ok(json!({"url": url, "headers": headers}))
}

fn image_request(input: &ImageRequestInput) -> HelperResult {
    let body =
        transform_image_generation_request(&input.model, &input.prompt, &input.optional_params);
    let encoded = serde_json::to_value(body)?;
    Ok(json!({"body": encoded}))
}

fn image_reply(input: &ImageReplyInput) -> HelperResult {
    let data = decode_dashscope_image_reply(input.body.as_bytes(), input.status, &input.headers)
        .map_err(image_failure)?;
    let encoded = serde_json::to_value(data)?;
    Ok(json!({"data": encoded}))
}

fn family_settings(input: &FamilySettingsInput) -> HelperResult {
    let api_base = input.api_base.as_deref();
    let api_key = input.api_key.as_deref();
    let dashscope = family_value(
        DashscopeFamily::Dashscope,
        &input.dashscope,
        api_base,
        api_key,
    )?;
    let qwencloud = family_value(
        DashscopeFamily::QwenCloud,
        &input.qwencloud,
        api_base,
        api_key,
    )?;
    let platform = family_value(
        DashscopeFamily::QwenAiPlatform,
        &input.qwen_ai_platform,
        api_base,
        api_key,
    )?;
    Ok(json!({
        "dashscope": dashscope,
        "qwencloud": qwencloud,
        "qwen_ai_platform": platform,
    }))
}

fn family_value(
    family: DashscopeFamily,
    step: &FamilyStep,
    api_base: Option<&str>,
    api_key: Option<&str>,
) -> HelperResult {
    let env = family_env(step);
    let image = image_config(family, &env);
    let embedding = embedding_config(family, &env);
    let RerankConfig {
        identity: rerank_identity,
        settings: rerank_settings,
    } = rerank_config(family, &env);
    let rerank = DashscopeRerankConfig {
        identity: rerank_identity,
        rerank_env_value: rerank_settings.env_value,
        default_rerank_url: rerank_settings.default_rerank_url,
    };
    let env_key = resolve_family_api_key(
        family,
        None,
        env.brand_key.as_ref(),
        env.fallback_key.as_ref(),
    );
    let chat = DashscopeChatConfig {
        policy: chat_policy(family, &env),
        env_api_key: env_key,
    };
    let argument_key = api_key.map(SecretString::from);
    let info = chat.provider_info(api_base, argument_key.as_ref());
    let chat_key = info.api_key.map(|key| key.expose_secret().clone());
    let image_headers = image
        .validate_environment(None, &Headers::new())
        .map_err(image_failure)?;
    let embedding_url = embedding.complete_url(api_base);
    let rerank_url = rerank.get_complete_url(api_base, RERANK_MODEL, None);
    let image_url = image.complete_url(api_base);
    let headers = serde_json::to_value(image_headers)?;
    Ok(json!({
        "chat": {"api_base": info.api_base, "api_key": chat_key},
        "embedding_url": embedding_url,
        "rerank_url": rerank_url,
        "image": {"url": image_url, "headers": headers},
    }))
}

fn family_env(step: &FamilyStep) -> FamilyEnv {
    FamilyEnv {
        base: step.base.clone(),
        brand_key: step.brand_key.as_deref().map(SecretString::from),
        fallback_key: step.fallback_key.as_deref().map(SecretString::from),
        rerank_base: step.rerank_base.clone(),
        image_base: step.image_base.clone(),
    }
}

fn family_key_and_message(input: &FamilyKeyAndMessageInput) -> HelperResult {
    let api_key = input.api_key.as_deref();
    let dashscope_key = input.dashscope_key.as_deref().map(SecretString::from);
    let qwencloud_key = input.qwencloud_key.as_deref().map(SecretString::from);
    let platform_key = input
        .qwen_ai_platform_key
        .as_deref()
        .map(SecretString::from);
    Ok(json!({
        "dashscope": key_and_message(DashscopeFamily::Dashscope, api_key, None, dashscope_key.as_ref()),
        "qwencloud": key_and_message(DashscopeFamily::QwenCloud, api_key, qwencloud_key.as_ref(), dashscope_key.as_ref()),
        "qwen_ai_platform": key_and_message(DashscopeFamily::QwenAiPlatform, api_key, platform_key.as_ref(), dashscope_key.as_ref()),
    }))
}

fn key_and_message(
    family: DashscopeFamily,
    api_key: Option<&str>,
    brand_key: Option<&SecretString>,
    dashscope_key: Option<&SecretString>,
) -> serde_json::Value {
    let key = resolve_family_api_key(family, api_key, brand_key, dashscope_key)
        .map(|key| key.expose_secret().clone());
    json!({"key": key, "message": missing_family_key_message(family)})
}

fn image_failure(error: DashscopeImageError) -> Box<HelperError> {
    match error {
        DashscopeImageError::Reply(provider) => failure(
            "BaseLLMException",
            Some(provider.status_code),
            provider.message,
        ),
        DashscopeImageError::MissingKey { message } => failure("ValueError", None, message),
        other @ DashscopeImageError::ParamType { .. } => {
            failure("TypeError", None, other.to_string())
        }
    }
}

fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

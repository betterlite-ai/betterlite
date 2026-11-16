use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::image_generation::decode::decode_image_reply;
use betterlite_openai::image_generation::defaults::apply_family_defaults;
use betterlite_openai::image_generation::family::ImageFamily;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::credentials::headers::redact_azure_auth_headers;
use crate::credentials::source::{CallableTokenProvider, CredentialSources};
use crate::endpoints::projection::LiteLlmParams;
use crate::image_transport::request::{
    Call, CallStyle, Connection, ImageClientConstructor, PollingSettings, execute,
};
use crate::image_transport::{ImageClock, ImageResponse};

use super::error::{ClassifiedError, ImageError};
use super::prepare::{PrepareContext, Prepared, prepare};
use super::selection::{AzureImageConfig, get_azure_image_generation_config};
use super::wire::{header_map, text_value, wire_map};

// AzureChatCompletion.image_generation's inputs, with litellm_params split into the keys the client reads
#[derive(Default)]
pub struct Generation<'data> {
    pub model: Option<&'data str>,
    pub prompt: &'data str,
    pub timeout: Option<f64>,
    pub optional_params: IndexMap<String, Json>,
    pub headers: Vec<(String, String)>,
    pub api_key: Option<&'data SecretString>,
    pub api_base: Option<&'data str>,
    pub api_version: Option<&'data str>,
    pub azure_ad_token: Option<&'data SecretString>,
    pub azure_ad_token_provider: Option<Arc<dyn CallableTokenProvider>>,
    pub base_model: Option<&'data str>,
    pub params: LiteLlmParams,
    pub delegate_async: bool,
    pub now_seconds: i64,
}

pub struct Runtime<'data, C, K> {
    pub endpoints: &'data EndpointSettings,
    pub environment: &'data dyn EnvironmentLookup,
    pub sources: &'data dyn CredentialSources,
    pub refresh_enabled: bool,
    pub polling_timeout: Duration,
    pub constructor: &'data C,
    pub clock: &'data K,
    pub logger: &'data mut dyn ImageGenerationLogger,
}

// B004's reply carries no hidden-params field, so the deployment's base model travels beside it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub reply: ImageGenerationReply,
    pub hidden_model: Option<String>,
}

pub async fn image_generation<C, K>(
    call: Generation<'_>,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<GeneratedImage, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let prepared = prepare(
        call,
        &PrepareContext {
            endpoints: runtime.endpoints,
            environment: runtime.environment,
            sources: runtime.sources,
            refresh_enabled: runtime.refresh_enabled,
        },
    )
    .map_err(ImageError::into_sync)?;
    match reply(&prepared, runtime).await {
        Ok(image) => Ok(image),
        Err(error) => {
            // the async leg logs its failure and re-raises, and this wrapper then wraps what it did not raise itself
            if prepared.async_mode {
                runtime.logger.post_call(PostCallRecord {
                    input: prepared.prompt.clone(),
                    original_response: Json::Text(error.to_string()),
                    data: Some(prepared.data.clone()),
                });
            }
            Err(ImageError::into_sync(error))
        }
    }
}

// The async leg applies the selected family's reply transform; the sync leg special-cases only MAI
async fn reply<C, K>(
    prepared: &Prepared,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<GeneratedImage, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let style = if prepared.async_mode {
        CallStyle::Async
    } else {
        CallStyle::Sync
    };
    let response = send(prepared, runtime, style).await?;
    let config = get_azure_image_generation_config(text_value(&prepared.data, "model")?.as_deref());
    if config == AzureImageConfig::Mai {
        // B554 owns the MAI reply transform, and a provider leaf cannot depend on betterlite-foundry
        return Err(ImageError::Classified(ClassifiedError {
            class: String::from("NotBuilt"),
            status: None,
            message: String::from("the MAI reply transform is B554"),
        }));
    }
    runtime.logger.post_call(PostCallRecord {
        input: prepared.prompt.clone(),
        original_response: serde_json::from_slice(&response.body)?,
        data: Some(prepared.data.clone()),
    });
    let mut reply = decode_image_reply(&response.body, prepared.now_seconds)?;
    if prepared.async_mode {
        let family = config.family().unwrap_or(ImageFamily::GptImage);
        apply_family_defaults(family, &mut reply, &prepared.data)?;
    }
    Ok(GeneratedImage {
        reply,
        hidden_model: prepared.hidden_model.clone(),
    })
}

async fn send<C, K>(
    prepared: &Prepared,
    runtime: &mut Runtime<'_, C, K>,
    style: CallStyle,
) -> Result<ImageResponse, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let headers = redact_azure_auth_headers(&prepared.headers)
        .into_iter()
        .map(|(name, value)| (name, Json::Text(value)))
        .collect();
    runtime.logger.pre_call(PreCallRecord {
        input: prepared.body_prompt.clone(),
        headers,
        api_base: prepared.url.clone(),
        data: prepared.data.clone(),
    })?;
    let api_version = match style {
        CallStyle::Sync => &prepared.argument_version,
        CallStyle::Async => &prepared.resolved_version,
    };
    let mut data = wire_map(&prepared.data);
    execute(
        runtime.constructor,
        Connection {
            client: None,
            timeout: prepared.timeout,
        },
        runtime.clock,
        PollingSettings {
            timeout: runtime.polling_timeout,
            style,
        },
        Call {
            api_base: &prepared.url,
            api_version,
            deployment_name: prepared.deployment.as_deref(),
            data: &mut data,
            headers: header_map(&prepared.headers)?,
        },
    )
    .await
    .map_err(ImageError::from)
}

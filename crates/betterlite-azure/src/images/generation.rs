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

use super::error::ImageError;
use super::mai::{MaiProjection, MaiReply};
use super::prepare::{PrepareContext, Prepared, prepare};
use super::selection::{AzureImageConfig, get_azure_image_generation_config};
use super::wire::{body_model, header_map, wire_map};

// AzureChatCompletion.image_generation's inputs, with litellm_params split into the keys the client reads
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
    pub mai: &'data dyn MaiProjection,
}

// B004's reply carries no hidden-params field, so the deployment's base model travels beside it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub reply: ImageGenerationReply,
    pub hidden_model: Option<String>,
}

impl<'data, C, K> Runtime<'data, C, K> {
    fn context(&self) -> PrepareContext<'data> {
        PrepareContext {
            endpoints: self.endpoints,
            environment: self.environment,
            sources: self.sources,
            refresh_enabled: self.refresh_enabled,
        }
    }
}

pub async fn image_generation<C, K>(
    call: Generation<'_>,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<GeneratedImage, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let prepared = prepare(call, &runtime.context()).map_err(ImageError::into_sync)?;
    if prepared.async_mode {
        // the async leg logs and re-raises, and this wrapper then wraps everything it did not raise itself
        return async_reply(&prepared, runtime)
            .await
            .map_err(ImageError::into_sync);
    }
    sync_reply(&prepared, runtime)
        .await
        .map_err(ImageError::into_sync)
}

// The sync leg applies the MAI transform only; every other family keeps B004's generic projection
async fn sync_reply<C, K>(
    prepared: &Prepared,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<GeneratedImage, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let response = send(prepared, runtime, CallStyle::Sync).await?;
    if get_azure_image_generation_config(body_model(&prepared.data)?) == AzureImageConfig::Mai {
        return project_mai(prepared, &response, runtime);
    }
    runtime.logger.post_call(PostCallRecord {
        input: prepared.prompt.clone(),
        original_response: serde_json::from_slice(&response.body)?,
        data: Some(prepared.data.clone()),
    });
    let reply = decode_image_reply(&response.body, prepared.now_seconds)?;
    Ok(GeneratedImage {
        reply,
        hidden_model: prepared.hidden_model.clone(),
    })
}

// The async leg re-throws the original failure after logging it, and image_generation wraps it afterwards
async fn async_reply<C, K>(
    prepared: &Prepared,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<GeneratedImage, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    match async_send(prepared, runtime).await {
        Ok(reply) => Ok(reply),
        Err(error) => {
            runtime.logger.post_call(PostCallRecord {
                input: prepared.prompt.clone(),
                original_response: Json::Text(error.to_string()),
                data: Some(prepared.data.clone()),
            });
            Err(error)
        }
    }
}

// The async leg applies the selected family's transform, which logs its own post_call before converting
async fn async_send<C, K>(
    prepared: &Prepared,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<GeneratedImage, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let response = send(prepared, runtime, CallStyle::Async).await?;
    let config = get_azure_image_generation_config(body_model(&prepared.data)?);
    if config == AzureImageConfig::Mai {
        return project_mai(prepared, &response, runtime);
    }
    runtime.logger.post_call(PostCallRecord {
        input: prepared.prompt.clone(),
        original_response: serde_json::from_slice(&response.body)?,
        data: Some(prepared.data.clone()),
    });
    let family = config.family().unwrap_or(ImageFamily::GptImage);
    let mut reply = decode_image_reply(&response.body, prepared.now_seconds)?;
    apply_family_defaults(family, &mut reply, &prepared.data)?;
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

fn project_mai<C, K>(
    prepared: &Prepared,
    response: &ImageResponse,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<GeneratedImage, ImageError> {
    let reply = runtime.mai.project(
        MaiReply {
            body: &response.body,
            status_code: response.status,
            data: &prepared.data,
        },
        runtime.logger,
    )?;
    Ok(GeneratedImage {
        reply,
        hidden_model: prepared.hidden_model.clone(),
    })
}

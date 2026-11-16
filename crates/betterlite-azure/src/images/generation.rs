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
use super::selection::{AzureImageConfig, image_config};
use super::target::{AzureClientParams, create_azure_base_url};
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

// The call after its pre_call: the logger, environment and credential sources stay out of the future
pub struct Begun {
    prepared: Prepared,
    url: String,
    config: AzureImageConfig,
}

pub fn begin<C, K>(
    call: Generation<'_>,
    runtime: &mut Runtime<'_, C, K>,
) -> Result<Begun, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let prepared = prepare_call(call, runtime)?;
    let body_model = text_value(&prepared.data, "model")?;
    let config = image_config(body_model.as_deref());
    let url = match target_url(&prepared, body_model.as_deref()) {
        Ok(url) => url,
        Err(error) => return Err(fail(&prepared, error, runtime.logger)),
    };
    if let Err(error) = pre_call(&prepared, &url, runtime.logger) {
        return Err(fail(&prepared, error, runtime.logger));
    }
    Ok(Begun {
        prepared,
        url,
        config,
    })
}

// What LiteLLM resolves before the async leg takes over: the body, the auth headers and the send inputs
fn prepare_call<C, K>(
    call: Generation<'_>,
    runtime: &Runtime<'_, C, K>,
) -> Result<Prepared, ImageError> {
    prepare(
        call,
        &PrepareContext {
            endpoints: runtime.endpoints,
            environment: runtime.environment,
            sources: runtime.sources,
            refresh_enabled: runtime.refresh_enabled,
        },
    )
    .map_err(ImageError::into_sync)
}

// B249's pre_call reads the prompt from the body and the auth headers with their values redacted
fn pre_call(
    prepared: &Prepared,
    url: &str,
    logger: &mut dyn ImageGenerationLogger,
) -> Result<(), ImageError> {
    let headers = redact_azure_auth_headers(&prepared.headers)
        .into_iter()
        .map(|(name, value)| (name, Json::Text(value)))
        .collect();
    logger
        .pre_call(PreCallRecord {
            input: prepared.body_prompt.clone(),
            headers,
            api_base: url.to_owned(),
            data: prepared.data.clone(),
        })
        .map_err(ImageError::Logging)
}

// only the transport call crosses the await point; the caller runs begin and finish around it
pub async fn send<C, K>(
    begun: &Begun,
    constructor: &C,
    clock: &K,
    polling_timeout: Duration,
) -> Result<ImageResponse, ImageError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let prepared = &begun.prepared;
    let (style, api_version) = if prepared.async_mode {
        (CallStyle::Async, prepared.resolved_version.as_str())
    } else {
        (CallStyle::Sync, prepared.argument_version.as_str())
    };
    execute(
        constructor,
        Connection {
            client: None,
            timeout: prepared.timeout,
        },
        clock,
        PollingSettings {
            timeout: polling_timeout,
            style,
        },
        Call {
            api_base: &begun.url,
            api_version,
            deployment_name: prepared.deployment.as_deref(),
            data: &mut wire_map(&prepared.data),
            headers: header_map(&prepared.headers)?,
        },
    )
    .await
    .map_err(ImageError::from)
}

// The async leg logs a failure as text and re-raises it; the sync leg wraps it and logs nothing
pub fn finish(
    begun: &Begun,
    sent: Result<ImageResponse, ImageError>,
    logger: &mut dyn ImageGenerationLogger,
) -> Result<GeneratedImage, ImageError> {
    let result = sent.and_then(|response| convert(begun, &response, logger));
    result.map_err(|error| fail(&begun.prepared, error, logger))
}

// The async leg applies the selected family's reply transform; the sync leg special-cases only MAI
fn convert(
    begun: &Begun,
    response: &ImageResponse,
    logger: &mut dyn ImageGenerationLogger,
) -> Result<GeneratedImage, ImageError> {
    let prepared = &begun.prepared;
    if !(200..300).contains(&response.status) {
        return Err(ImageError::Status {
            status: response.status,
            body: response_text(&response.body),
        });
    }
    if begun.config == AzureImageConfig::Mai {
        // B554 owns the MAI reply transform, and a provider leaf cannot depend on betterlite-foundry
        return Err(ImageError::Classified(ClassifiedError {
            class: String::from("NotBuilt"),
            status: None,
            message: String::from("the MAI reply transform is B554"),
        }));
    }
    logger.post_call(PostCallRecord {
        input: prepared.prompt.clone(),
        original_response: serde_json::from_slice(&response.body)?,
        data: Some(prepared.data.clone()),
    });
    let mut reply = decode_image_reply(&response.body, prepared.now_seconds)?;
    if prepared.async_mode {
        let family = begun.config.family().unwrap_or(ImageFamily::GptImage);
        apply_family_defaults(family, &mut reply, &prepared.data)?;
    }
    Ok(GeneratedImage {
        reply,
        hidden_model: prepared.hidden_model.clone(),
    })
}

// The async leg builds its URL from data's model on both sides, the sync leg from the deployment
fn target_url(prepared: &Prepared, body_model: Option<&str>) -> Result<String, ImageError> {
    let deployment = prepared.deployment.as_deref();
    let (model, base_model) = if prepared.async_mode {
        (deployment.or(body_model), body_model)
    } else {
        (deployment, prepared.hidden_model.as_deref())
    };
    create_azure_base_url(&client_params(prepared), model, base_model).map_err(ImageError::from)
}

fn client_params(prepared: &Prepared) -> AzureClientParams<'_> {
    AzureClientParams {
        azure_endpoint: prepared.azure_endpoint.as_deref(),
        base_url: prepared.base_url.as_deref(),
        api_version: Some(prepared.resolved_version.as_str()),
    }
}

// The sync leg wraps the failure; the async leg logs the prompt and the error text, then re-raises it as it was
fn fail(
    prepared: &Prepared,
    error: ImageError,
    logger: &mut dyn ImageGenerationLogger,
) -> ImageError {
    if !prepared.async_mode {
        return error.into_sync();
    }
    logger.post_call(PostCallRecord {
        input: prepared.prompt.clone(),
        original_response: Json::Text(error.to_string()),
        data: Some(prepared.data.clone()),
    });
    error
}

// an error body that is not UTF-8 has no text to show, the same as the image transport's helper
fn response_text(body: &[u8]) -> String {
    String::from_utf8(body.to_vec()).unwrap_or_else(|error| {
        let length = error.as_bytes().len();
        format!("{length} bytes that are not UTF-8")
    })
}

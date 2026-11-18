use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::image_variations::credentials::VariationPolicies;
use betterlite_core::api::image_variations::error::{ImageVariationError, VariationException};
use betterlite_core::api::image_variations::{
    ImageVariationCall, ImageVariations, PendingVariation, TopazVariationExecutor,
    TopazVariationRequest, VariationAnswer, VariationOptions, VariationResponseFormat,
    aimage_variation, image_variation,
};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::result::{Failure, Nullable};
use super::vector_query_embedding::outbound_client;

// image_variation's parameters, which the provider call receives as optional_params={} no matter what
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub image: Image,
    pub model: Option<String>,
    pub n: Option<i64>,
    pub response_format: Option<ResponseFormat>,
    pub size: Option<String>,
    pub user: Option<String>,
    #[serde(default)]
    pub asynchronous: bool,
    #[serde(default)]
    pub litellm_params: Params,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub filename: String,
    pub content: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Params {
    pub api_key: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub timeout_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    Url,
    B64Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub success: Option<Success>,
    pub failure: Option<Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Success {
    pub created: i64,
    pub data: Vec<Datum>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Datum {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl From<&Call> for ImageVariationCall {
    fn from(input: &Call) -> Self {
        Self {
            image: ImageFile {
                filename: Some(input.image.filename.clone()),
                content: input.image.content.clone().into_bytes(),
                content_type: input.image.content_type.clone(),
            },
            model: input.model.clone(),
            options: VariationOptions {
                n: input.n.unwrap_or(1),
                response_format: match input.response_format {
                    Some(ResponseFormat::B64Json) => VariationResponseFormat::B64Json,
                    Some(ResponseFormat::Url) | None => VariationResponseFormat::Url,
                },
                size: input.size.clone(),
                user: input.user.clone(),
            },
            api_key: input.litellm_params.api_key.clone().map(SecretString::from),
            api_base: None,
            custom_llm_provider: input.litellm_params.custom_llm_provider.clone(),
            timeout: input.litellm_params.timeout_seconds,
            async_call: input.asynchronous,
            metadata: ImageParams::new(),
        }
    }
}

impl CallKind for Call {
    async fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<SdkResult, SdkHttpError> {
        execute(self, api_base, env).await
    }
}

// the harness owns the mock origin, so it takes the place of both providers' configured api_base
async fn execute(
    input: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let client = outbound_client(env)?;
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let mut settings = loader
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?
        .sections
        .image_variations;
    settings.api_base = Some(api_base.to_owned());
    settings.topaz_api_base = Some(api_base.to_owned());
    let call = ImageVariationCall::from(input);
    let catalog = BundledCatalog::default().get();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let topaz = NoTopaz;
    let service = ImageVariations {
        settings: &settings,
        resolution: &resolution,
        transport: &client,
        topaz: &topaz,
        policies: VariationPolicies::default(),
        identify: Uuid::new_v4,
        clock: || 0,
    };
    if input.asynchronous {
        match aimage_variation(&service, &call).await {
            Ok(answer) => success(answer),
            Err(error) => exception_failure(&error),
        }
    } else {
        match image_variation(&service, &call).await {
            Ok(answer) => success(answer),
            Err(error) => Ok(error_failure(&error)),
        }
    }
}

// B046 supplies Topaz through a B010 operation and is not part of the SDK comparison
struct NoTopaz;

impl TopazVariationExecutor for NoTopaz {
    fn execute(
        &self,
        _request: TopazVariationRequest,
    ) -> Result<Option<PendingVariation<'_>>, ImageVariationError> {
        Ok(None)
    }
}

fn success(answer: VariationAnswer) -> Result<SdkResult, SdkHttpError> {
    let VariationAnswer::Response(response) = answer else {
        return Err(SdkHttpError::Invalid(
            "the image variation call returned no image response".to_owned(),
        ));
    };
    let data = response
        .data
        .unwrap_or_default()
        .into_iter()
        .map(|datum| Datum {
            b64_json: datum.b64_json,
            revised_prompt: datum.revised_prompt,
            url: datum.url,
        })
        .collect();
    Ok(SdkResult::ImageVariations {
        value: Box::new(Outcome {
            success: Some(Success {
                created: response.created,
                data,
            }),
            failure: None,
        }),
    })
}

// image_variation's own ValueErrors carry no status, type, param or code
fn error_failure(error: &ImageVariationError) -> SdkResult {
    let (class, status) = match error {
        ImageVariationError::InvalidProvider { .. }
        | ImageVariationError::UnknownModelInfo { .. }
        | ImageVariationError::MissingKey { .. }
        | ImageVariationError::MissingBase { .. }
        | ImageVariationError::InvalidTimeout { .. } => ("ValueError", 0),
        ImageVariationError::Resolution(_) => ("BadRequestError", 400),
        ImageVariationError::OpenAi(source) => {
            (class_for_status(source.status_code), source.status_code)
        }
        ImageVariationError::Topaz(source) => ("Exception", source.status_code.unwrap_or(0)),
    };
    SdkResult::Failure(Failure {
        class: class.to_owned(),
        status,
        kind: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// aimage_variation re-raises through exception_type, so the class and its fields come from the mapping
fn exception_failure(error: &VariationException) -> Result<SdkResult, SdkHttpError> {
    let metadata = error.metadata();
    Ok(SdkResult::Failure(Failure {
        class: class_name(error.class())?.to_owned(),
        status: metadata.status_code,
        kind: field(metadata.error_type),
        param: field(metadata.param),
        code: Some(metadata.openai_code),
        message: error.message.clone(),
    }))
}

fn class_for_status(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500 => "InternalServerError",
        502 => "BadGatewayError",
        503 => "ServiceUnavailableError",
        504 => "Timeout",
        _ => "APIError",
    }
}

fn class_name(class: &ErrorClass) -> Result<&'static str, SdkHttpError> {
    match class {
        ErrorClass::Authentication(_) => Ok("AuthenticationError"),
        ErrorClass::NotFound(_) => Ok("NotFoundError"),
        ErrorClass::PermissionDenied(_) => Ok("PermissionDeniedError"),
        ErrorClass::BadRequest(_) => Ok("BadRequestError"),
        ErrorClass::UnprocessableEntity(_) => Ok("UnprocessableEntityError"),
        ErrorClass::ContextWindowExceeded(_) => Ok("ContextWindowExceededError"),
        ErrorClass::ContentPolicyViolation(_) => Ok("ContentPolicyViolationError"),
        ErrorClass::ServiceUnavailable(_) => Ok("ServiceUnavailableError"),
        ErrorClass::BadGateway(_) => Ok("BadGatewayError"),
        ErrorClass::InternalServer(_) => Ok("InternalServerError"),
        ErrorClass::Timeout(_) => Ok("Timeout"),
        ErrorClass::RateLimit(_) => Ok("RateLimitError"),
        ErrorClass::Api(_) => Ok("APIError"),
        ErrorClass::ApiConnection(_) => Ok("APIConnectionError"),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}

fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

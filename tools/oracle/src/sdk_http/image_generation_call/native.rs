use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::image_generation_call::Settings;
use betterlite_core::api::image_generation_call::call::{DEFAULT_TIMEOUT, ImageGenerationCall};
use betterlite_core::api::image_generation_call::error::ImageGenerationException;
use betterlite_core::api::image_generation_call::operations::CustomProviderSlot;
use betterlite_core::api::image_generation_call::plan::{
    CatalogIdentity, ImageGenerationAnswer, ImageGenerationPlan, image_generation,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::super::vector_query_embedding::outbound_client;
use super::operations::Operations;
use super::report::{failure, logging_state, result_json, unknown_provider};
use super::sends::custom::CustomImage;
use super::{Call, MOCK, Outcome};

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let environment = substituted(env, api_base);
    let settings = load_settings(call, api_base, &environment)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let source = outbound_client(&environment)?;
    let catalog = BundledCatalog::default().get();
    let known_model_names = BTreeSet::new();
    let custom_providers: BTreeSet<String> = call.custom_providers.iter().cloned().collect();
    let slots = custom_slots(&call.custom_providers, &source);
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let identity = CatalogIdentity {
        context: &resolution,
    };
    let operations = Operations { source: &source };
    let plan = ImageGenerationPlan {
        identity: &identity,
        operations: &operations,
        settings: &settings,
        environment: &environment,
        custom_providers: &custom_providers,
        custom_provider_map: &slots,
        azure_ad_token_provider: None,
        drop_params: false,
        redactor: &redactor,
    };
    let mut state = logging_state(&redactor)?;
    let answer = image_generation(&plan, core_call(call, api_base), &mut state).await;
    Ok(SdkResult::ImageGenerationCall {
        value: Box::new(to_outcome(answer)?),
    })
}

// every custom provider the case registers answers through the same mock-bound handler
fn custom_slots(providers: &[String], source: &ProxiedClient) -> Vec<CustomProviderSlot> {
    providers
        .iter()
        .map(|provider| CustomProviderSlot {
            provider: provider.clone(),
            handler: Some(Arc::new(CustomImage {
                source: source.clone(),
            })),
        })
        .collect()
}

fn substituted(env: &BTreeMap<String, String>, api_base: &str) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| (name.clone(), value.replace(MOCK, api_base)))
        .collect()
}

// the case's litellm module globals, read through the same config section the proxy loads
fn load_settings(
    call: &Call,
    api_base: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Settings, SdkHttpError> {
    let litellm_settings =
        serde_json::to_string(&call.litellm_settings).map_err(SdkHttpError::Serialize)?;
    let document = format!("{{\"litellm_settings\":{litellm_settings}}}").replace(MOCK, api_base);
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&document)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    Ok(config.sections.image_generation_call.clone())
}

fn core_call(call: &Call, api_base: &str) -> ImageGenerationCall {
    ImageGenerationCall {
        model: call.model.clone(),
        prompt: call.prompt.clone(),
        image_count: call.image_count,
        quality: call.quality.clone(),
        response_format: call.response_format.clone(),
        size: call.size.clone(),
        style: call.style.clone(),
        user: call.user.clone(),
        timeout: call.timeout.unwrap_or(DEFAULT_TIMEOUT),
        api_key: call.api_key.clone().map(SecretString::from),
        api_base: Some(api_base.to_owned()),
        api_version: call.api_version.clone(),
        custom_llm_provider: call.custom_llm_provider.clone(),
        client_kind: None,
        now_seconds: 0,
        kwargs: call_kwargs(call),
    }
}

// the kwargs the Python side passes only when the case sets them
fn call_kwargs(call: &Call) -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    if !call.headers.is_empty() {
        kwargs.insert(String::from("headers"), headers_json(&call.headers));
    }
    if let Some(extra_headers) = &call.extra_headers {
        kwargs.insert(String::from("extra_headers"), headers_json(extra_headers));
    }
    if let Some(organization) = &call.organization {
        kwargs.insert(
            String::from("organization"),
            Json::Text(organization.clone()),
        );
    }
    if let Some(base_model) = &call.base_model {
        kwargs.insert(String::from("base_model"), Json::Text(base_model.clone()));
    }
    if let Some(mock_response) = &call.mock_response {
        kwargs.insert(
            String::from("mock_response"),
            Json::Text(mock_response.clone()),
        );
    }
    if call.aimg_generation {
        kwargs.insert(String::from("aimg_generation"), Json::Bool(true));
    }
    kwargs
}

fn headers_json(headers: &BTreeMap<String, String>) -> Json {
    Json::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}

fn to_outcome(
    answer: Result<ImageGenerationAnswer, ImageGenerationException>,
) -> Result<Outcome, SdkHttpError> {
    match answer {
        Ok(generated) => result_json(&generated.response),
        Err(ImageGenerationException::Mapped { class, message }) => Ok(Outcome {
            result: None,
            failure: Some(failure(&class, &message)?),
        }),
        Err(ImageGenerationException::UnknownProvider { message }) => Ok(Outcome {
            result: None,
            failure: Some(unknown_provider(&message)),
        }),
    }
}

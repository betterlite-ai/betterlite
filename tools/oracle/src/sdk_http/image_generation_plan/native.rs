use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::image_generation_call::Settings;
use betterlite_core::api::image_generation_call::call::{DEFAULT_TIMEOUT, ImageGenerationCall};
use betterlite_core::api::image_generation_call::error::ImageGenerationException;
use betterlite_core::api::image_generation_call::plan::{
    CatalogIdentity, ImageGenerationPlan, image_generation,
};
use betterlite_core::api::image_generation_plan::error::AimageGenerationError;
use betterlite_core::api::image_generation_plan::invocation::{Invocation, ModelPosition};
use betterlite_core::api::image_generation_plan::{
    AimageGenerationPlan, PlanEntry, aimage_generation,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::ImageResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::super::image_generation_call::Call as ImageCall;
use super::super::result::kinds::SdkResult;
use super::super::vector_query_embedding::outbound_client;
use super::operations::Operations;
use super::report::{failure, logging_state, result_json, unknown_provider};
use super::{Call, MOCK, Outcome, SyncMode};

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let world = World::new(call, api_base, env)?;
    let resolution = world.resolution();
    let identity = CatalogIdentity {
        context: &resolution,
    };
    let operations = Operations {
        source: &world.source,
    };
    // the plan B998's own oracle adapter builds, minus the provider pieces this adapter does not wire
    let plan = ImageGenerationPlan {
        identity: &identity,
        operations: &operations,
        settings: &world.settings,
        environment: &world.variables,
        custom_providers: &world.custom_providers,
        custom_provider_map: &[],
        azure_ad_token_provider: None,
        drop_params: false,
        redactor: &world.redactor,
    };
    let entry = PlanEntry { plan: &plan };
    let facade = AimageGenerationPlan {
        entry: &entry,
        resolution: &resolution,
        redactor: &world.redactor,
    };
    let mut state = logging_state(&world.redactor)?;
    let outcome = match call.sync_mode {
        // image_generation answers the answer type; the wrapper above it keeps only the response
        SyncMode::Sync => projected(
            image_generation(&plan, core_call(&call.call, api_base), &mut state)
                .await
                .map(|answer| answer.response)
                .map_err(|failure| match failure {
                    ImageGenerationException::Mapped { class, message } => {
                        AimageGenerationError::Raised { class, message }
                    }
                    ImageGenerationException::UnknownProvider { message } => {
                        AimageGenerationError::UnknownProvider { message }
                    }
                }),
        )?,
        SyncMode::Asynchronous => projected(
            aimage_generation(
                &facade,
                keyword_invocation(&call.call, api_base),
                &mut state,
            )
            .await,
        )?,
    };
    Ok(SdkResult::ImageGenerationPlan {
        value: Box::new(outcome),
    })
}

// the case's environment and litellm module globals, read through the sections the proxy loads
struct World {
    settings: Settings,
    redactor: Redactor,
    source: ProxiedClient,
    catalog: Arc<ModelCatalog>,
    names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    variables: BTreeMap<String, String>,
}

impl World {
    fn new(
        call: &Call,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<Self, SdkHttpError> {
        let variables = substituted(env, api_base);
        let settings = load_settings(call, api_base, &variables)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let source = outbound_client(&variables)?;
        let catalog = BundledCatalog::default().get();
        let custom_providers = call.call.custom_providers.iter().cloned().collect();
        Ok(Self {
            settings,
            redactor,
            source,
            catalog,
            names: BTreeSet::new(),
            custom_providers,
            variables,
        })
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.custom_providers,
            environment: &self.variables,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

// the response or the SDK exception either entry point answers, as B998's own adapter encodes it
fn projected(
    result: Result<ImageResponse, AimageGenerationError>,
) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(response) => result_json(&response),
        Err(AimageGenerationError::Raised { class, message }) => failure(&class, &message),
        Err(AimageGenerationError::UnknownProvider { message }) => Ok(unknown_provider(&message)),
        Err(AimageGenerationError::MissingModel) => Err(SdkHttpError::Invalid(String::from(
            "the image_generation_plan call requires a model",
        ))),
    }
}

// the wrapper's keyword selection, with no positional argument to read
fn keyword_invocation(call: &ImageCall, api_base: &str) -> Invocation {
    Invocation {
        call: core_call(call, api_base),
        model_position: ModelPosition::Keyword,
    }
}

fn substituted(env: &BTreeMap<String, String>, api_base: &str) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| (name.clone(), value.replace(MOCK, api_base)))
        .collect()
}

// the case's litellm_settings, read through the same config section the proxy loads
fn load_settings(
    call: &Call,
    api_base: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Settings, SdkHttpError> {
    let litellm_settings =
        serde_json::to_string(&call.call.litellm_settings).map_err(SdkHttpError::Serialize)?;
    let document = format!("{{\"litellm_settings\":{litellm_settings}}}").replace(MOCK, api_base);
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&document)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    Ok(config.sections.image_generation_call.clone())
}

fn core_call(call: &ImageCall, api_base: &str) -> ImageGenerationCall {
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
fn call_kwargs(call: &ImageCall) -> IndexMap<String, Json> {
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

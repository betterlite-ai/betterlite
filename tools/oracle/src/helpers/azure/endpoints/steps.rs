use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::cloudflare::{CloudflareRequest, init_cloudflare_gateway_client};
use betterlite_azure::endpoints::get_client::{GetClientRequest, get_azure_openai_client};
use betterlite_azure::endpoints::initialize::{
    EndpointError, InitializeRequest, initialize_azure_sdk_client,
};
use betterlite_azure::endpoints::projection::{ClientKind, SuppliedClient};
use betterlite_config::sections::azure_endpoints::{
    AZURE_DEFAULT_API_VERSION, DEFAULT_MAX_RETRIES, Settings,
};
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::fixture::{Cache, Sources, callable};
use super::reports;
use super::{
    ClientStep, CloudflareStep, Defaults, DynamicStep, InitializeStep, Input, SuppliedKind,
    SuppliedStep,
};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::DefaultApiVersion(steps) | Input::AzureInitializerCredentials(steps) => {
            initialize_steps(steps)
        }
        Input::CloudflareBaseModel(steps) => cloudflare_steps(steps),
        Input::SeparateTokenClientIdentity(steps)
        | Input::AzureSdkClientMode(steps)
        | Input::AzureV1ClientProjection(steps) => client_steps(steps),
        Input::DynamicApiVersion(steps) => Ok(json!({
            "steps": steps.iter().map(dynamic_step).collect::<Vec<Value>>()
        })),
    }
}

fn dynamic_step(step: &DynamicStep) -> Value {
    match betterlite_azure::endpoints::dynamic::check_dynamic_azure_params(
        step.api_version.as_deref(),
        step.custom_query.as_ref(),
    ) {
        Ok(changed) => json!({"changed": changed}),
        Err(betterlite_azure::endpoints::dynamic::DynamicVersionError::MissingApiVersion) => {
            json!({"error": "KeyError"})
        }
    }
}

fn settings(configured: Option<&Defaults>) -> Settings {
    configured.map_or_else(
        || Settings {
            default_max_retries: DEFAULT_MAX_RETRIES,
            azure_default_api_version: AZURE_DEFAULT_API_VERSION.to_owned(),
        },
        |values| Settings {
            default_max_retries: values.default_max_retries,
            azure_default_api_version: values.azure_default_api_version.clone(),
        },
    )
}

fn initialize_steps(steps: &[InitializeStep]) -> HelperResult {
    let mut reports = Vec::with_capacity(steps.len());
    for step in steps {
        reports.push(initialize_step(step)?);
    }
    Ok(json!({"steps": reports}))
}

fn initialize_step(step: &InitializeStep) -> Result<Value, Box<HelperError>> {
    let sources = Sources::new(&step.sources);
    let settings = settings(step.defaults.as_ref());
    let request = InitializeRequest {
        params: step.params.clone(),
        provider: step
            .provider
            .as_ref()
            .map(|provider| callable(provider.script.clone(), provider.identity)),
        api_key: step
            .api_key
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        api_base: step.api_base.clone(),
        model_name: None,
        api_version: step.api_version.clone(),
        is_async: step.is_async,
        session: step.session.session(),
        refresh_enabled: step.refresh,
        settings: &settings,
        environment: &step.environment,
        sources: &sources,
    };
    let mut refresh = RefreshProviderCache::new(&sources);
    let inputs = initialize_azure_sdk_client(&request, &mut refresh)
        .map_err(|error| endpoint_error(&error))?;
    Ok(reports::initialize_report(
        &inputs,
        step.provider.is_some(),
        &sources,
    ))
}

fn client_steps(steps: &[ClientStep]) -> HelperResult {
    let mut reports = Vec::with_capacity(steps.len());
    for step in steps {
        reports.push(client_step(step)?);
    }
    Ok(json!({"steps": reports}))
}

fn client_step(step: &ClientStep) -> Result<Value, Box<HelperError>> {
    let sources = Sources::new(&step.sources);
    let settings = settings(step.defaults.as_ref());
    let request = GetClientRequest {
        api_key: step
            .api_key
            .as_ref()
            .map(|value| SecretString::from(value.clone())),
        api_base: step.api_base.clone(),
        api_version: step.api_version.clone(),
        model: None,
        is_async: step.is_async,
        params: step.params.clone(),
        provider: step
            .provider
            .as_ref()
            .map(|provider| callable(provider.script.clone(), provider.identity)),
        supplied: step.supplied.as_ref().map(supplied_client),
        settings: &settings,
        environment: &step.environment,
        refresh_enabled: step.refresh,
        session: step.session.session(),
        sources: &sources,
    };
    let mut refresh = RefreshProviderCache::new(&sources);
    let mut cache = Cache::for_choice(step.cached);
    let outcome = get_azure_openai_client(&request, &mut cache, &mut refresh)
        .map_err(|error| endpoint_error(&error))?;
    let owned = cache.sets.last().map(|(_, _, owned)| *owned);
    Ok(reports::client_report(&outcome, owned, &sources.calls()))
}

fn supplied_client(step: &SuppliedStep) -> SuppliedClient {
    SuppliedClient {
        kind: supplied_kind(step.kind),
        custom_query: step.custom_query.clone(),
    }
}

const fn supplied_kind(kind: SuppliedKind) -> ClientKind {
    match kind {
        SuppliedKind::Azure => ClientKind::AzureOpenAi,
        SuppliedKind::AsyncAzure => ClientKind::AsyncAzureOpenAi,
        SuppliedKind::Standard => ClientKind::OpenAi,
        SuppliedKind::AsyncStandard => ClientKind::AsyncOpenAi,
    }
}

fn cloudflare_steps(steps: &[CloudflareStep]) -> HelperResult {
    let mut reports = Vec::with_capacity(steps.len());
    for step in steps {
        reports.push(cloudflare_step(step)?);
    }
    Ok(json!({"steps": reports}))
}

fn cloudflare_step(step: &CloudflareStep) -> Result<Value, Box<HelperError>> {
    let sources = Sources::new(&step.sources);
    let api_key = step
        .api_key
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let azure_ad_token = step
        .azure_ad_token
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let provider = step
        .provider
        .as_ref()
        .map(|provider| callable(provider.script.clone(), provider.identity).provider);
    let supplied = step.client.as_ref().map(supplied_client);
    let request = CloudflareRequest {
        api_base: &step.api_base,
        model: &step.model,
        api_version: &step.api_version,
        max_retries: step.max_retries,
        timeout: step.timeout,
        params: &step.params,
        api_key: api_key.as_ref(),
        azure_ad_token: azure_ad_token.as_ref(),
        token_provider: provider.as_ref(),
        shared_session: step.shared_session,
        is_async: step.is_async,
        client: supplied.as_ref(),
        environment: &step.environment,
        sources: &sources,
    };
    let outcome =
        init_cloudflare_gateway_client(&request).map_err(|error| endpoint_error(&error))?;
    Ok(reports::cloudflare_report(&outcome, &sources.calls()))
}

fn endpoint_error(error: &EndpointError) -> Box<HelperError> {
    Box::new(HelperError {
        class: error.class().to_owned(),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

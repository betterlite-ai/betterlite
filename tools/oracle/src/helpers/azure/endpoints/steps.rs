use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::cloudflare::{CloudflareRequest, init_cloudflare_gateway_client};
use betterlite_azure::endpoints::get_client::initialize_request;
use betterlite_azure::endpoints::get_client::{GetClientRequest, get_azure_openai_client};
use betterlite_azure::endpoints::initialize::{EndpointError, initialize_azure_sdk_client};
use betterlite_azure::endpoints::projection::{ClientKind, SuppliedClient};
use betterlite_config::sections::azure_endpoints::Settings;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::fixture::{Cache, Sources, callable};
use super::reports;
use super::{ClientStep, CloudflareStep, Defaults, DynamicStep, Input, SuppliedKind, SuppliedStep};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::DefaultApiVersion(steps) | Input::AzureInitializerCredentials(steps) => {
            run_steps(steps, initialize_step)
        }
        Input::CloudflareBaseModel(steps) => run_steps(steps, cloudflare_step),
        Input::SeparateTokenClientIdentity(steps)
        | Input::AzureSdkClientMode(steps)
        | Input::AzureV1ClientProjection(steps) => run_steps(steps, client_step),
        Input::DynamicApiVersion(steps) => Ok(json!({
            "steps": steps.iter().map(dynamic_step).collect::<Vec<Value>>()
        })),
    }
}

fn run_steps<S>(steps: &[S], one: fn(&S) -> Result<Value, EndpointError>) -> HelperResult {
    steps
        .iter()
        .map(one)
        .collect::<Result<Vec<Value>, _>>()
        .map(|reports| json!({"steps": reports}))
        .map_err(|error| {
            Box::new(HelperError {
                class: error.class().to_owned(),
                status: error.status(),
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
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

impl From<&Defaults> for Settings {
    fn from(values: &Defaults) -> Self {
        Self {
            default_max_retries: values.default_max_retries,
            azure_default_api_version: values.azure_default_api_version.clone(),
        }
    }
}

struct Prepared {
    sources: Sources,
    settings: Settings,
}

impl Prepared {
    fn new(step: &ClientStep) -> Self {
        Self {
            sources: Sources::new(&step.sources),
            settings: step
                .defaults
                .as_ref()
                .map_or_else(Settings::default, Settings::from),
        }
    }
}

fn initialize_step(step: &ClientStep) -> Result<Value, EndpointError> {
    let prep = Prepared::new(step);
    let request = initialize_request(&client_request(step, &prep));
    let mut refresh = RefreshProviderCache::new(&prep.sources);
    let inputs = initialize_azure_sdk_client(&request, &mut refresh)?;
    Ok(reports::initialize_report(&inputs, step, &prep.sources))
}

fn client_step(step: &ClientStep) -> Result<Value, EndpointError> {
    let prep = Prepared::new(step);
    let mut refresh = RefreshProviderCache::new(&prep.sources);
    let mut cache = Cache::for_choice(step.cached);
    let request = client_request(step, &prep);
    let client = get_azure_openai_client(&request, &mut cache, &mut refresh)?;
    Ok(reports::client_report(&client, &cache, &prep.sources))
}

fn client_request<'data>(
    step: &'data ClientStep,
    prepared: &'data Prepared,
) -> GetClientRequest<'data> {
    GetClientRequest {
        api_key: step.api_key.clone(),
        provider: step
            .provider
            .as_ref()
            .map(|provider| callable(provider.script.clone(), provider.identity)),
        supplied: step.supplied.as_ref().map(supplied_client),
        api_base: step.api_base.clone(),
        api_version: step.api_version.clone(),
        model: None,
        is_async: step.is_async,
        params: step.params.clone(),
        settings: &prepared.settings,
        environment: &step.environment,
        refresh_enabled: step.refresh,
        session: step.session.session(),
        sources: &prepared.sources,
    }
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

fn cloudflare_step(step: &CloudflareStep) -> Result<Value, EndpointError> {
    let sources = Sources::new(&step.sources);
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
        api_key: step.api_key.as_ref(),
        azure_ad_token: step.azure_ad_token.as_ref(),
        token_provider: provider.as_ref(),
        shared_session: step.shared_session,
        is_async: step.is_async,
        client: supplied.as_ref(),
        environment: &step.environment,
        sources: &sources,
    };
    let outcome = init_cloudflare_gateway_client(&request)?;
    Ok(reports::cloudflare_report(&outcome, &sources.calls()))
}

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::file_content_stream::FileContentStream;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::FilesApi;
use chrono::Utc;
use uuid::Uuid;

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::kinds::SdkResult;

use super::super::wire::{ContentDispatch, Handler};
use super::super::{Call, run_steps};
use super::anthropic_settings;

const HANDLER_HEADER: (&str, &str) = ("x-supplied-client", "http-handler");

pub(crate) async fn send_steps(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let resolution = model_resolution(&catalog, &names, env);
    let content = FileContentStream {
        dispatch: ContentDispatch {
            openai: FilesApi {
                transport: client.clone(),
                boundaries: Uuid::new_v4,
            },
            client,
            anthropic: anthropic_settings(&config.sections.anthropic_credentials),
            environment: env.clone(),
        },
        settings: config.sections.resource_sdk_settings,
        resolution: &resolution,
        clock: || Utc::now().fixed_offset(),
        call_ids: Uuid::new_v4,
    };
    let handler = Handler {
        headers: BTreeMap::from([(HANDLER_HEADER.0.to_owned(), HANDLER_HEADER.1.to_owned())]),
    };
    run_steps(&content, call, api_base, &handler).await
}

// the catalog and environment the resolver reads, held by send_steps for the call's lifetime
fn model_resolution<'deps>(
    catalog: &'deps ModelCatalog,
    names: &'deps BTreeSet<String>,
    env: &'deps BTreeMap<String, String>,
) -> ResolutionContext<'deps> {
    ResolutionContext {
        catalog,
        known_model_names: names,
        custom_providers: names,
        environment: env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

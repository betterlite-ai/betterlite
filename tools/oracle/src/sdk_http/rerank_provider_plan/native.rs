use super::super::error::SdkHttpError;
use super::super::result::SyncDiagnostic;
use super::super::result::kinds::SdkResult;
use super::input::{core_call, settings_document};
use super::providers::direct;
use super::report::{Diagnostics, failure, logging_state};
use super::{Call, Failure, MOCK, Outcome};
use betterlite_config::api::rerank_provider_plan::{RerankSettings, loads_rerank_settings};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_core::api::rerank_provider_plan::plan::{
    CatalogIdentity, RerankException, RerankPlan, rerank,
};
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use operations::Operations;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::PoisonError;
use tracing::subscriber::set_default;
use tracing_subscriber::registry;
mod operations;
const FIXTURE_NOW: &str = "2026-01-01T00:00:05+00:00";
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let environment = substituted_environment(env, api_base);
    let document = settings_document(call, api_base)?;
    let settings = loads_rerank_settings(&document, &environment)
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let loaded = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&document)
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let operations = build_operations(&environment, &loaded)?;
    let catalog = BundledCatalog::default().get();
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
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
    let plan = build_plan(&identity, &operations, &settings, &environment);
    let mut state = logging_state(&operations.redactor)?;
    let diagnostics = Diagnostics::default();
    let subscription = set_default(registry().with(diagnostics.clone()));
    let outcome = rerank(&plan, core_call(call, api_base)?, &mut state).await;
    drop(subscription);
    let diagnostics =
        std::mem::take(&mut *diagnostics.0.lock().unwrap_or_else(PoisonError::into_inner));
    let value = to_outcome(outcome, diagnostics)?;
    Ok(SdkResult::RerankProviderPlan {
        value: Box::new(value),
    })
}
fn substituted_environment(
    env: &BTreeMap<String, String>,
    api_base: &str,
) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| (name.clone(), value.replace(MOCK, api_base)))
        .collect()
}
fn build_operations<'settings>(
    environment: &'settings BTreeMap<String, String>,
    config: &'settings Config,
) -> Result<Operations<'settings>, SdkHttpError> {
    Ok(Operations {
        environment,
        config,
        source: direct(environment)?,
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
        now: DateTime::parse_from_rfc3339(FIXTURE_NOW)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?,
    })
}
fn build_plan<'a, Identity>(
    identity: &'a Identity,
    operations: &'a Operations<'a>,
    settings: &'a RerankSettings,
    environment: &'a BTreeMap<String, String>,
) -> RerankPlan<'a, Identity, Operations<'a>> {
    RerankPlan {
        identity,
        operations,
        settings,
        environment,
        redactor: &operations.redactor,
    }
}
fn to_outcome(
    result: Result<RerankResponse, RerankException>,
    diagnostics: Vec<SyncDiagnostic>,
) -> Result<Outcome, SdkHttpError> {
    let (value, raised) = match result {
        Ok(response) => (
            Some(serde_json::to_value(&response).map_err(SdkHttpError::Serialize)?),
            None,
        ),
        Err(exception) => (None, Some(failure(&exception)?)),
    };
    Ok(Outcome {
        result: value,
        failure: raised,
        diagnostics,
    })
}

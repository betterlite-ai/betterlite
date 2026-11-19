use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::cost_adjustments::Settings as AdjustmentSettings;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::cost::completion::lookup::{CatalogLookup, LookupSettings};
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use jiff::tz::TimeZoneDatabase;

use super::deployments::{DeploymentSet, DeploymentSetError};
use super::error::EstimateError;
use super::estimate::estimate;
use crate::cost_service::{CostService, CostServiceError, Policies, name_normalizers};
use crate::proxy_request_contracts::ContractError;
use crate::proxy_request_contracts::cost_estimate::CostEstimateRequest;

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Deployment(#[from] DeploymentSetError),
    #[error(transparent)]
    Service(#[from] CostServiceError),
    #[error(transparent)]
    Request(#[from] ContractError),
    #[error(transparent)]
    Estimate(#[from] EstimateError),
    #[error("the fixture runtime could not be built: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("the fixture value could not be converted: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn service_from(model_cost: &[u8]) -> Result<CostService<ProviderFormulas>, FixtureError> {
    let settings = LookupSettings {
        known_model_names: BTreeSet::new(),
        custom_providers: BTreeSet::new(),
        environment: BTreeMap::new(),
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
        normalizers: name_normalizers(),
    };
    let catalog = ModelCatalog::parse(model_cost, UnknownFields::Warn)?;
    let lookup = CatalogLookup::new(model_cost, Arc::new(catalog), settings)
        .map_err(CostServiceError::Lookup)?;
    Ok(CostService::new(
        lookup,
        &AdjustmentSettings::default(),
        Policies::native(),
        TimeZoneDatabase::from_env(),
    )?)
}

pub fn quote(records: &[JsonObject], request: &JsonValue) -> Result<JsonValue, FixtureError> {
    let deployments = DeploymentSet::from_records(records)?;
    let service = service_from(&(BundledCatalog::default().asset)()?)?;
    let request = CostEstimateRequest::from_value(request)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let response = runtime.block_on(estimate(
        &service,
        Some(&deployments),
        &request,
        SystemTime::now(),
    ))?;
    Ok(serde_json::to_value(response)?)
}

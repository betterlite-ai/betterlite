use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use thiserror::Error;

use crate::endpoints::passthrough_config_reads::db_read::database_endpoints;
use crate::endpoints::passthrough_config_reads::ports::ConfigEndpointsSource;
use crate::endpoints::passthrough_config_reads::ports::GeneralSettingsReader;
use crate::endpoints::passthrough_config_reads::record::EndpointRecordError;
use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use crate::passthrough::route_registration::PassThroughEndpoint;
use crate::passthrough::route_registration::RouteRegistrationError;
use crate::passthrough::route_registration::SharedPassThroughRoutes;
// ast-grep-ignore: no-long-names
use crate::passthrough::route_registration::initialize_pass_through_endpoints;

use super::convert::guardrails_object;
use super::convert::json_object;

#[derive(Debug, Error)]
pub enum InitializeError {
    #[error("a stored pass-through endpoint cannot be decoded")]
    Record(#[source] EndpointRecordError),
    #[error("a configured pass-through endpoint cannot be decoded")]
    Config(#[source] serde_json::Error),
    #[error("the pass-through routes cannot be registered")]
    Registration(#[source] RouteRegistrationError),
}

// ast-grep-ignore: no-long-names
pub async fn initialize_pass_through_endpoints_in_db(
    store: &dyn GeneralSettingsReader,
    registry: &SharedPassThroughRoutes,
    config: &dyn ConfigEndpointsSource,
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<(), InitializeError> {
    let mut endpoints = stored_endpoints(store).await?;
    endpoints.extend(configured_endpoints(config)?);
    // ast-grep-ignore: no-long-names
    initialize_pass_through_endpoints(registry, &endpoints, None, secrets, environment)
        .map_err(InitializeError::Registration)
}

// LiteLLM catches every failure of the stored read, and a stored value that is not a list holds no endpoints it can register
async fn stored_endpoints(
    store: &dyn GeneralSettingsReader,
) -> Result<Vec<PassThroughEndpoint>, InitializeError> {
    match database_endpoints(store, None, None).await {
        Ok(records) => Ok(records.iter().map(registration_endpoint).collect()),
        Err(EndpointRecordError::NotAList) => Ok(Vec::new()),
        Err(error) => Err(InitializeError::Record(error)),
    }
}

// LiteLLM hands the config file's raw entries to the same registration as the stored ones, so an absent auth stays absent
fn configured_endpoints(
    config: &dyn ConfigEndpointsSource,
) -> Result<Vec<PassThroughEndpoint>, InitializeError> {
    let Some(entries) = config.configured_endpoints() else {
        return Ok(Vec::new());
    };
    let mut endpoints = Vec::with_capacity(entries.len());
    for entry in entries {
        let text = serde_json::to_string(entry).map_err(InitializeError::Config)?;
        endpoints.push(serde_json::from_str(&text).map_err(InitializeError::Config)?);
    }
    Ok(endpoints)
}

// the stored entry as the registration reads it: every field present, with the model's own auth value
fn registration_endpoint(endpoint: &PassThroughGenericEndpoint) -> PassThroughEndpoint {
    PassThroughEndpoint {
        id: endpoint.id.clone(),
        path: Some(endpoint.path.clone()),
        target: Some(endpoint.target.clone()),
        headers: Some(json_object(&endpoint.headers)),
        forward_headers: None,
        merge_query_params: None,
        default_query_params: Some(json_object(&endpoint.default_query_params)),
        include_subpath: Some(Json::Bool(endpoint.include_subpath.0)),
        auth: Some(Json::Bool(endpoint.auth.0)),
        methods: endpoint.methods.clone(),
        cost_per_request: Some(endpoint.cost_per_request),
        guardrails: guardrails_object(endpoint.guardrails.as_ref()),
        timeout: endpoint.timeout,
    }
}

use super::error::HealthProbeError;
use super::params::{HealthProbeParams, Param};
use betterlite_auth::resolved_key::factories::{SystemPrincipal, system_principal};
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_metadata_safety::metadata::MetadataField;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::add_principal_metadata;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
pub const SERVICE_ACCOUNT: &str = "litellm-internal-health-check";
const METADATA_VARIABLE_NAME: &str = "litellm_metadata";
const TAGS_KEY: &str = "tags";
pub fn add_tracking_information(
    mut params: HealthProbeParams,
) -> Result<HealthProbeParams, HealthProbeError> {
    let principal = health_principal()?;
    let mut data = RequestData::new();
    let mut metadata = IndexMap::new();
    metadata.insert(
        TAGS_KEY.to_owned(),
        Json::List(vec![Json::Text(SERVICE_ACCOUNT.to_owned())]),
    );
    data.insert(METADATA_VARIABLE_NAME.to_owned(), Json::Object(metadata));
    add_principal_metadata(&mut data, MetadataField::LitellmMetadata, &principal)
        .map_err(|source| HealthProbeError::Tracking { source })?;
    let Some(Json::Object(metadata)) = data.shift_remove(METADATA_VARIABLE_NAME) else {
        return Err(HealthProbeError::TrackingMetadata);
    };
    params.litellm_metadata = Param::Value(Json::Object(metadata));
    Ok(params)
}
fn health_principal() -> Result<PrincipalFacts, HealthProbeError> {
    let principal = system_principal(&SystemPrincipal::HealthCheck)
        .map_err(|source| HealthProbeError::Principal { source })?;
    let encoded = serde_json::to_string(&principal)
        .map_err(|source| HealthProbeError::PrincipalEncode { source })?;
    serde_json::from_str(&encoded).map_err(|source| HealthProbeError::PrincipalEncode { source })
}

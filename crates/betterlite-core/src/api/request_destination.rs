pub mod destination;
pub mod error;
pub mod model;
pub mod parameters;

mod tests;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::destination::{
    DestinationField, reject_url_valued_destination, reject_url_valued_destinations,
};
use crate::api::request_destination::error::RequestDestinationError;
use crate::api::request_destination::model::{
    InferenceModelKind, ModelDefaults, resolve_inference_model,
};
use crate::api::request_destination::parameters::{
    populate_request_parameters, tags_from_request_body,
};

pub type RequestData = IndexMap<String, Json>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DestinationSettings {
    pub model_defaults: ModelDefaults,
    pub allowed_hosts: Vec<AllowedDestination>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestDestinationInput {
    pub data: RequestData,
    pub path_params: Option<RequestData>,
    pub query_params: IndexMap<String, String>,
    pub route_path: String,
    pub kind: InferenceModelKind,
    pub cli_model: Option<String>,
    pub endpoint_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestDestinationResult {
    pub data: RequestData,
    pub model: String,
    pub tags: Vec<String>,
}

pub fn resolve_request_destination(
    input: &RequestDestinationInput,
    settings: &DestinationSettings,
) -> Result<RequestDestinationResult, RequestDestinationError> {
    let mut data = populate_request_parameters(
        &input.data,
        input.path_params.as_ref(),
        &input.query_params,
        &input.route_path,
    );
    reject_url_valued_destinations(&data, &settings.allowed_hosts)?;
    if let Some(endpoint_model) = input.endpoint_model.as_deref() {
        reject_url_valued_destination(
            DestinationField::Model,
            endpoint_model,
            &settings.allowed_hosts,
        )?;
    }
    let body_model = match data.get("model") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_other) => return Err(RequestDestinationError::ModelNotText),
    };
    let model = resolve_inference_model(
        body_model.as_deref(),
        &settings.model_defaults,
        input.cli_model.as_deref(),
        input.endpoint_model.as_deref(),
        input.kind,
    )
    .ok_or(RequestDestinationError::MissingModel)?;
    data.insert("model".to_owned(), Json::Text(model.clone()));
    let tags = tags_from_request_body(&data);
    Ok(RequestDestinationResult { data, model, tags })
}

use std::fmt;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_http::url_policy::destination::{candidates, destination_allowed};
use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::api::request_destination::RequestData;
use crate::api::request_destination::error::RequestDestinationError;

// LiteLLM's _URL_DESTINATION_REQUEST_FIELDS, in the order it checks them
pub const URL_DESTINATION_FIELDS: [(DestinationField, &str); 2] = [
    (DestinationField::Model, "model"),
    (DestinationField::FileId, "file_id"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationField {
    Model,
    FileId,
}

impl fmt::Display for DestinationField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Model => "model",
            Self::FileId => "file_id",
        };
        f.write_str(name)
    }
}

// a comma part or the text after its first slash may smuggle a URL into a provider-prefixed model
pub fn reject_url_valued_destination(
    field: DestinationField,
    value: &str,
    allowed_hosts: &[AllowedDestination],
) -> Result<(), RequestDestinationError> {
    for candidate in candidates(value) {
        let lowered = candidate.to_lowercase();
        let url_valued = lowered.starts_with("http://") || lowered.starts_with("https://");
        if !url_valued || destination_allowed(candidate, allowed_hosts) {
            continue;
        }
        return Err(RequestDestinationError::UrlValuedDestination { field });
    }
    Ok(())
}

pub fn reject_url_valued_destinations(
    data: &RequestData,
    allowed_hosts: &[AllowedDestination],
) -> Result<(), RequestDestinationError> {
    for (field, key) in URL_DESTINATION_FIELDS {
        if let Some(Json::Text(value)) = data.get(key) {
            reject_url_valued_destination(field, value, allowed_hosts)?;
        }
    }
    Ok(())
}

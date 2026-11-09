use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::newrelic_extraction::sources::EventSources;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestParameters {
    pub temperature: Option<Json>,
    pub max_tokens: Option<Json>,
}

#[must_use]
pub fn from_sources(sources: &EventSources<'_>) -> RequestParameters {
    let selected = sources
        .standard_logging_object
        .map_or(sources.kwargs.optional_params, |payload| {
            Some(&payload.model_parameters)
        });
    let Some(source) = selected else {
        return RequestParameters::default();
    };
    RequestParameters {
        temperature: parameter(source, "temperature"),
        max_tokens: parameter(source, "max_tokens"),
    }
}

fn parameter(source: &IndexMap<String, Json>, name: &str) -> Option<Json> {
    match source.get(name) {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    }
}

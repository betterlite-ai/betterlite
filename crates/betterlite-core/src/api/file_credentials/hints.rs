use serde::Deserialize;

use crate::api::file_routing_ids::routing::decode_model_from_file_id;

// The model-name sources a files route reads besides the id's embedded model, each None when the route saw none.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelHints {
    pub body: Option<String>,
    pub query: Option<String>,
    pub header: Option<String>,
}

// extract_model_from_sources: the embedded model, then body.model, then query.model, then the header, with an empty body or query falling through
#[must_use]
pub fn extract_model_from_sources(
    file_id: &str,
    hints: &ModelHints,
) -> (Option<String>, Option<String>) {
    let embedded = decode_model_from_file_id(file_id);
    let from_param = hints
        .body
        .clone()
        .filter(|model| !model.is_empty())
        .or_else(|| hints.query.clone().filter(|model| !model.is_empty()))
        .or_else(|| hints.header.clone());
    (embedded, from_param)
}

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_metadata::params::LitellmParams;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallState {
    pub response_headers: Option<IndexMap<String, String>>,
    pub litellm_params: Option<LitellmParams>,
    pub response_cost: Option<Json>,
}

pub fn surface_response_headers(state: &mut CallState, response_hidden: Option<&Json>) {
    if state.response_headers.is_some() {
        return;
    }
    let Some(headers) = provider_response_headers(response_hidden) else {
        return;
    };
    state.response_headers = Some(headers);
}

pub fn merge_response_hidden_params(state: &mut CallState, response_hidden: Option<&Json>) {
    let Some(Json::Object(hidden)) = response_hidden else {
        return;
    };
    // the pinned setup adds no metadata key for an empty hidden-params mapping
    if hidden.is_empty() {
        return;
    }
    let response_cost = state.response_cost.clone();
    let Some(params) = state.litellm_params.as_mut() else {
        return;
    };
    let mut merged = hidden.clone();
    if matches!(merged.get("response_cost"), None | Some(Json::Null))
        && let Some(cost) = response_cost
    {
        merged.insert("response_cost".to_owned(), cost);
    }
    let mut metadata = params.metadata.take().unwrap_or_default();
    metadata.insert("hidden_params".to_owned(), Json::Object(merged));
    params.metadata = Some(metadata);
}

fn provider_response_headers(hidden: Option<&Json>) -> Option<IndexMap<String, String>> {
    let Json::Object(fields) = hidden? else {
        return None;
    };
    let Some(Json::Object(entries)) = fields.get("headers") else {
        return None;
    };
    let mut headers = IndexMap::with_capacity(entries.len());
    for (name, value) in entries {
        let Json::Text(text) = value else {
            return None;
        };
        headers.insert(name.clone(), text.clone());
    }
    Some(headers)
}

mod tests;

use indexmap::IndexMap;

use super::support::{NodeAllocator, as_mapping};
use crate::spend_sanitize::UNKNOWN_MODEL;
use crate::spend_sanitize::raw_model::scrub_raw_model_text;
use crate::spend_sanitize::value::{SpendMapping, SpendValue};

pub fn placeholder_request_body(
    litellm_params: &SpendMapping,
    model_group: &str,
    raw_model: &str,
    nodes: &mut NodeAllocator,
) -> SpendMapping {
    let Some(proxy) = litellm_params
        .entries
        .get("proxy_server_request")
        .and_then(as_mapping)
    else {
        return litellm_params.clone();
    };
    let Some(body) = proxy.entries.get("body").and_then(as_mapping) else {
        return litellm_params.clone();
    };
    let placeholdered = if body.entries.contains_key("model") {
        let mut entries = body.entries.clone();
        entries.insert(
            "model".to_owned(),
            SpendValue::Text(UNKNOWN_MODEL.to_owned()),
        );
        SpendMapping {
            node: nodes.allocate(),
            entries,
        }
    } else {
        body.clone()
    };
    let new_body = placeholder_request_metadata(&placeholdered, model_group, raw_model, nodes);
    let mut proxy_entries = proxy.entries.clone();
    proxy_entries.insert("body".to_owned(), SpendValue::Mapping(new_body));
    let new_proxy = SpendMapping {
        node: nodes.allocate(),
        entries: proxy_entries,
    };
    let mut params = litellm_params.entries.clone();
    params.insert(
        "proxy_server_request".to_owned(),
        SpendValue::Mapping(new_proxy),
    );
    SpendMapping {
        node: nodes.allocate(),
        entries: params,
    }
}

pub fn placeholder_request_metadata(
    request_body: &SpendMapping,
    model_group: &str,
    raw_model: &str,
    nodes: &mut NodeAllocator,
) -> SpendMapping {
    let Some(metadata) = request_body.entries.get("metadata").and_then(as_mapping) else {
        return request_body.clone();
    };
    let mut entries = metadata.entries.clone();
    if entries.contains_key("model_group") {
        entries.insert(
            "model_group".to_owned(),
            SpendValue::Text(model_group.to_owned()),
        );
    }
    if let Some(SpendValue::Mapping(error_information)) = metadata.entries.get("error_information")
    {
        entries.insert(
            "error_information".to_owned(),
            SpendValue::Mapping(scrub_error_information(error_information, raw_model, nodes)),
        );
    }
    SpendMapping {
        node: nodes.allocate(),
        entries,
    }
}

fn scrub_error_information(
    information: &SpendMapping,
    raw_model: &str,
    nodes: &mut NodeAllocator,
) -> SpendMapping {
    let mut entries = IndexMap::with_capacity(information.entries.len());
    for (key, value) in &information.entries {
        let scrubbed = match (key.as_str(), value.as_text()) {
            ("error_message" | "traceback", Some(text)) => {
                SpendValue::Text(scrub_raw_model_text(text, raw_model))
            }
            _ => value.clone(),
        };
        entries.insert(key.clone(), scrubbed);
    }
    SpendMapping {
        node: nodes.allocate(),
        entries,
    }
}

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

mod tests;

#[must_use]
pub fn request_labels_from_params(params: Option<&Json>) -> Option<BTreeMap<String, String>> {
    let Some(Json::Object(fields)) = params else {
        return None;
    };
    for key in ["metadata", "litellm_metadata"] {
        let Some(Json::Object(metadata)) = fields.get(key) else {
            continue;
        };
        let Some(Json::Object(requester)) = metadata.get("requester_metadata") else {
            continue;
        };
        let labels = text_labels(requester.iter());
        if !labels.is_empty() {
            return Some(labels);
        }
    }
    None
}

#[must_use]
pub fn pop_vertex_request_labels(
    optional_params: &mut Option<Json>,
    litellm_params: Option<&Json>,
) -> Option<BTreeMap<String, String>> {
    let mut labels = match optional_params.as_mut() {
        Some(Json::Object(fields)) => fields.shift_remove("labels").and_then(label_map),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            None
        }
    };
    if labels.as_ref().is_none_or(BTreeMap::is_empty) {
        labels = request_labels_from_params(litellm_params);
    }
    labels.filter(|found| !found.is_empty())
}

fn label_map(raw: Json) -> Option<BTreeMap<String, String>> {
    match raw {
        Json::Object(fields) => Some(text_labels(fields.iter())),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn text_labels<'fields, Pairs>(fields: Pairs) -> BTreeMap<String, String>
where
    Pairs: Iterator<Item = (&'fields String, &'fields Json)>,
{
    fields
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

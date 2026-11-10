use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

mod tests;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GroundingRequests {
    pub web_search_requests: Option<i64>,
    pub google_maps_grounding_requests: Option<i64>,
}

#[must_use]
pub fn calculate_grounding_requests(grounding_metadata: &[Json]) -> GroundingRequests {
    let mut web_queries = BTreeSet::new();
    let mut maps_queries = BTreeSet::new();
    let mut has_maps = false;
    for item in grounding_metadata {
        let kinds = chunk_kinds(item);
        let maps_item = kinds.contains("maps") || has_widget_token(item);
        has_maps = has_maps || maps_item;
        let target = if maps_item && !kinds.contains("web") {
            &mut maps_queries
        } else {
            &mut web_queries
        };
        target.extend(queries(item));
    }
    let web_count = i64::try_from(web_queries.len()).ok();
    let maps_count = i64::try_from(maps_queries.len()).ok();
    // a Maps lookup bills the prompt even when the provider reports no query for it
    let maps_requests = has_maps.then(|| maps_count.unwrap_or_default().max(1));
    GroundingRequests {
        web_search_requests: web_count.filter(|count| *count > 0),
        google_maps_grounding_requests: maps_requests,
    }
}

fn chunk_kinds(item: &Json) -> BTreeSet<&str> {
    let mut kinds = BTreeSet::new();
    let Some(Json::List(chunks)) = field(item, "groundingChunks") else {
        return kinds;
    };
    for chunk in chunks {
        if let Json::Object(fields) = chunk {
            kinds.extend(fields.keys().map(String::as_str));
        }
    }
    kinds
}

fn queries(item: &Json) -> Vec<&str> {
    let Some(Json::List(entries)) = field(item, "webSearchQueries") else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            Json::Text(text) if !text.is_empty() => Some(text.as_str()),
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_) => None,
        })
        .collect()
}

fn has_widget_token(item: &Json) -> bool {
    match field(item, "googleMapsWidgetContextToken") {
        Some(Json::Text(token)) => !token.is_empty(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => false,
    }
}

fn field<'json>(value: &'json Json, name: &str) -> Option<&'json Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

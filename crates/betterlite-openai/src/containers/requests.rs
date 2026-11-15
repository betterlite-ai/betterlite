use serde_json::{Map, Value};

use crate::api::container_contracts::config::ContainerListQuery;

pub const SUPPORTED_PARAMS: [&str; 4] = ["name", "expires_after", "file_ids", "extra_headers"];

#[must_use]
pub fn create_request(name: &str, params: &Map<String, Value>) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert(String::from("name"), Value::String(name.to_owned()));
    for (key, value) in params {
        if key != "extra_headers" {
            body.insert(key.clone(), value.clone());
        }
    }
    body
}

#[must_use]
pub fn list_query(query: &ContainerListQuery<'_>) -> Map<String, Value> {
    let mut params = Map::new();
    if let Some(after) = query.after {
        params.insert(String::from("after"), Value::String(after.to_owned()));
    }
    if let Some(limit) = query.limit {
        params.insert(String::from("limit"), Value::String(limit.to_string()));
    }
    if let Some(order) = query.order {
        params.insert(String::from("order"), Value::String(order.to_owned()));
    }
    if let Some(extra) = query.extra_query {
        for (key, value) in extra {
            params.insert(key.clone(), value.clone());
        }
    }
    params
}

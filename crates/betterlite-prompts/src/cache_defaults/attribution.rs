use std::collections::BTreeMap;

use betterlite_context::request_records::GatewayCacheAttribution;
use betterlite_context::request_records::record::RequestRecord;

use crate::arguments::json::{Json, Object};
use crate::cache_accounting::constants::{
    GATEWAY_CACHE_METADATA_KEY, INJECTED_FOR_EVERY_DEPLOYMENT,
};

const BUCKETS: [&str; 2] = ["litellm_metadata", "metadata"];

// Attribution reads a RequestRecord; this hands it the two keys its mark reads, not the whole payload.
pub fn record_gateway_injection(
    params: &mut Object,
    added: i32,
    injected_for_every_deployment: bool,
) {
    if added <= 0_i32 {
        return;
    }
    let deployment = model_info_id(params).cloned();
    let mut record = attribution_view(params, deployment.as_ref());
    GatewayCacheAttribution::record(&mut record, added, injected_for_every_deployment);
    write_back(params, &record, deployment.as_ref());
}

fn model_info_id(params: &Object) -> Option<&Json> {
    match params.get("model_info")? {
        Json::Object(info) => info.get("id"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn attribution_view(params: &Object, deployment: Option<&Json>) -> RequestRecord {
    let mut fields = BTreeMap::new();
    for name in BUCKETS {
        let Some(bucket) = params.get(name).and_then(Json::as_object) else {
            continue;
        };
        let mut view = BTreeMap::new();
        if let Some(marker) = bucket.get(GATEWAY_CACHE_METADATA_KEY) {
            view.insert(GATEWAY_CACHE_METADATA_KEY.to_owned(), record_of(marker));
        }
        fields.insert(name.to_owned(), RequestRecord::Object(view));
    }
    let mut info = BTreeMap::new();
    if let Some(id) = deployment {
        info.insert("id".to_owned(), record_of(id));
    }
    fields.insert("model_info".to_owned(), RequestRecord::Object(info));
    RequestRecord::Object(fields)
}

// A written marker is the deployment id or the empty every-deployment sentinel, so the original id is the value.
fn write_back(params: &mut Object, record: &RequestRecord, deployment: Option<&Json>) {
    let RequestRecord::Object(fields) = record else {
        return;
    };
    for name in BUCKETS {
        let Some(RequestRecord::Object(view)) = fields.get(name) else {
            continue;
        };
        let Some(marker) = view.get(GATEWAY_CACHE_METADATA_KEY) else {
            continue;
        };
        let sentinel = matches!(marker, RequestRecord::String(text) if text.as_str() == INJECTED_FOR_EVERY_DEPLOYMENT);
        let written = if sentinel {
            Json::Text(INJECTED_FOR_EVERY_DEPLOYMENT.to_owned())
        } else {
            deployment
                .cloned()
                .unwrap_or(Json::Text(INJECTED_FOR_EVERY_DEPLOYMENT.to_owned()))
        };
        let Some(Json::Object(bucket)) = params.get_mut(name) else {
            continue;
        };
        bucket.insert(GATEWAY_CACHE_METADATA_KEY.to_owned(), written);
        return;
    }
}

// The marker rule only distinguishes the every-deployment sentinel; every other value is re-read from the original JSON.
fn record_of(value: &Json) -> RequestRecord {
    serde_json::to_value(value)
        .ok()
        .and_then(|encoded| serde_json::from_value(encoded).ok())
        .unwrap_or(RequestRecord::Null)
}

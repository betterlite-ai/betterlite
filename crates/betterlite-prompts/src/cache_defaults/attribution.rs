use std::collections::BTreeMap;

use betterlite_context::request_records::GatewayCacheAttribution;
use betterlite_context::request_records::record::RequestRecord;

use crate::arguments::json::{Json, Object};

const MARKER: &str = "litellm_gateway_injected_cache";
const BUCKETS: [&str; 2] = ["litellm_metadata", "metadata"];

// Attribution reads a RequestRecord; this hands it the three keys its mark reads, not the whole payload.
pub fn record_gateway_injection(params: &mut Object, added: usize) {
    let count = i32::try_from(added).unwrap_or(i32::MAX);
    if count <= 0_i32 {
        return;
    }
    let mut record = attribution_view(params);
    GatewayCacheAttribution::record(&mut record, count, false);
    write_back(params, &record);
}

fn attribution_view(params: &Object) -> RequestRecord {
    let mut fields = BTreeMap::new();
    for name in BUCKETS {
        let Some(bucket) = params.get(name).and_then(Json::as_object) else {
            continue;
        };
        let mut view = BTreeMap::new();
        if let Some(marker) = bucket.get(MARKER) {
            view.insert(MARKER.to_owned(), record_value(marker));
        }
        fields.insert(name.to_owned(), RequestRecord::Object(view));
    }
    let model_info = match params.get("model_info") {
        Some(Json::Object(info)) => {
            let mut view = BTreeMap::new();
            if let Some(id) = info.get("id") {
                view.insert("id".to_owned(), record_value(id));
            }
            RequestRecord::Object(view)
        }
        Some(value) => record_value(value),
        None => RequestRecord::Null,
    };
    fields.insert("model_info".to_owned(), model_info);
    RequestRecord::Object(fields)
}

fn write_back(params: &mut Object, record: &RequestRecord) {
    let RequestRecord::Object(fields) = record else {
        return;
    };
    for name in BUCKETS {
        let Some(RequestRecord::Object(view)) = fields.get(name) else {
            continue;
        };
        let Some(marker) = view.get(MARKER) else {
            continue;
        };
        let Some(Json::Object(bucket)) = params.get_mut(name) else {
            continue;
        };
        bucket.insert(MARKER.to_owned(), json_value(marker));
        return;
    }
}

// The view only carries JSON values, so anything else coming back becomes a null.
fn json_value(value: &RequestRecord) -> Json {
    match value {
        RequestRecord::Null | RequestRecord::Handle(_) => Json::Null,
        RequestRecord::Bool(flag) => Json::Bool(*flag),
        RequestRecord::Signed(number) => Json::Number(serde_json::Number::from(*number)),
        RequestRecord::Unsigned(number) => Json::Number(serde_json::Number::from(*number)),
        RequestRecord::Decimal(number) => number_json(number.to_string()),
        RequestRecord::String(text) => Json::Text(text.clone()),
        RequestRecord::Array(items) => Json::List(items.iter().map(json_value).collect()),
        RequestRecord::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), json_value(item)))
                .collect(),
        ),
    }
}

fn number_json(text: String) -> Json {
    text.parse::<serde_json::Number>()
        .map_or_else(|_| Json::Text(text), Json::Number)
}

fn record_value(value: &Json) -> RequestRecord {
    match value {
        Json::Null => RequestRecord::Null,
        Json::Bool(flag) => RequestRecord::Bool(*flag),
        Json::Text(text) => RequestRecord::String(text.clone()),
        Json::Number(number) => number_record(number),
        Json::List(items) => RequestRecord::Array(items.iter().map(record_value).collect()),
        Json::Object(fields) => RequestRecord::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), record_value(item)))
                .collect(),
        ),
    }
}

fn number_record(number: &serde_json::Number) -> RequestRecord {
    if let Some(value) = number.as_i64() {
        return RequestRecord::Signed(value);
    }
    if let Some(value) = number.as_u64() {
        return RequestRecord::Unsigned(value);
    }
    serde_json::from_value(serde_json::Value::Number(number.clone())).unwrap_or(RequestRecord::Null)
}

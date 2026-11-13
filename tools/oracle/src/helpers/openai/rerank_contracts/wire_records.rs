use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankRequest, RerankResponse};
use betterlite_openai::api::rerank_contracts::wire::{
    RerankContract, RerankRecordKind, decode_rerank_contracts,
};
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use serde_json::{Value, json};

use super::WireParams;

const fn limits() -> WireDecodeLimits {
    WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    }
}

fn decode_request(payload: &Value) -> Option<RerankRequest> {
    let bytes = serde_json::to_vec(payload).ok()?;
    match decode_rerank_contracts(&bytes, RerankRecordKind::Request, limits()).ok()? {
        RerankContract::Request(record) => Some(record),
        RerankContract::Response(_) => None,
    }
}

pub(super) fn decode_response(payload: &Value) -> Option<RerankResponse> {
    let bytes = serde_json::to_vec(payload).ok()?;
    match decode_rerank_contracts(&bytes, RerankRecordKind::Response, limits()).ok()? {
        RerankContract::Response(record) => Some(record),
        RerankContract::Request(_) => None,
    }
}

fn accessor(response: &RerankResponse, key: &str) -> Value {
    let item = response.item(key).map_or_else(
        |_unknown| json!({"ok": false}),
        |value| json!({"ok": true, "value": value}),
    );
    let field = response.field(key);
    json!({
        "contains": field.is_some(),
        "item": item,
        "get": field.unwrap_or_else(|| Json::List(vec![Json::Text(String::from("__absent__"))])),
    })
}

fn request_outcome(payload: &Value) -> Value {
    decode_request(payload).map_or_else(
        || json!({"ok": false}),
        |record| {
            json!({
                "ok": true,
                "dump": serde_json::to_value(record).unwrap_or(Value::Null),
            })
        },
    )
}

fn response_outcome(payload: &Value, keys: &[String]) -> Value {
    let Some(response) = decode_response(payload) else {
        return json!({"ok": false});
    };
    let accessors: BTreeMap<String, Value> = keys
        .iter()
        .map(|key| (key.clone(), accessor(&response, key)))
        .collect();
    json!({
        "ok": true,
        "dump": serde_json::to_value(&response).unwrap_or(Value::Null),
        "accessors": accessors,
    })
}

pub(super) fn run(params: &WireParams) -> Value {
    json!({
        "requests": params.requests.iter().map(request_outcome).collect::<Vec<_>>(),
        "responses": params
            .responses
            .iter()
            .map(|payload| response_outcome(payload, &params.accessor_keys))
            .collect::<Vec<_>>(),
    })
}

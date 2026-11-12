#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::json;

use super::ToolUsage;
use crate::cost_usage::CostUsage;
use crate::cost_usage::server_tool_projection::ServerToolUse;
use crate::decisions::Json;

fn details(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn usage() -> ToolUsage {
    ToolUsage {
        cost: CostUsage {
            prompt_tokens: 7,
            completion_tokens: 2,
            total_tokens: 9,
            prompt_details: None,
            completion_details: None,
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        server_tool_use: Some(ServerToolUse {
            web_search_requests: Some(0),
            tool_search_requests: None,
            browser_open_requests: None,
        }),
        server_side_details: Some(details(&[(
            "web_search_calls",
            Json::Number(serde_json::Number::from(3_i64)),
        )])),
    }
}

#[test]
fn a_tool_usage_round_trips_through_json() {
    let original = usage();
    let encoded = serde_json::to_value(&original).expect("tool usage serializes");
    assert_eq!(encoded["cost"]["prompt_tokens"], json!(7_i64));
    assert_eq!(
        encoded["server_side_tool_usage_details"]["web_search_calls"],
        json!(3_i64)
    );
    let decoded: ToolUsage = serde_json::from_value(encoded).expect("tool usage deserializes");
    assert_eq!(decoded, original);
}

#[test]
fn absent_server_fields_take_their_defaults() {
    let decoded: ToolUsage = serde_json::from_value(json!({
        "cost": {"prompt_tokens": 1_i64, "completion_tokens": 2_i64, "total_tokens": 3_i64},
    }))
    .expect("a partial usage deserializes");
    assert_eq!(decoded.cost.prompt_tokens, 1);
    assert_eq!(decoded.server_tool_use, None);
    assert_eq!(decoded.server_side_details, None);
}

#[test]
fn an_empty_usage_takes_every_default() {
    let decoded: ToolUsage =
        serde_json::from_value(json!({})).expect("an empty usage deserializes");
    assert_eq!(decoded, ToolUsage::default());
}

#[test]
fn replacing_the_server_tool_use_keeps_the_cost_and_side_details() {
    let original = usage();
    let replacement = ServerToolUse {
        web_search_requests: Some(4),
        tool_search_requests: Some(1),
        browser_open_requests: None,
    };
    let repaired = original.with_server_tool_use(replacement);
    assert_eq!(repaired.cost, original.cost);
    assert_eq!(repaired.server_side_details, original.server_side_details);
    assert_eq!(repaired.server_tool_use, Some(replacement));
    assert_eq!(
        original
            .server_tool_use
            .expect("the original keeps zero")
            .web_search_requests,
        Some(0)
    );
}

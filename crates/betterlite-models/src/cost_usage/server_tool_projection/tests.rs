#![cfg(test)]

use indexmap::IndexMap;
use serde_json::Number;

use super::{ServerToolProjection, ServerToolRecord, ServerToolUse, web_search_requests};
use crate::decisions::Json;

fn mapping(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn parsed_record_keeps_an_absent_counter_absent() {
    let usage = ServerToolUse::default();
    assert_eq!(
        web_search_requests(Some(ServerToolRecord::Parsed(&usage))),
        None
    );
}

#[test]
fn parsed_record_keeps_a_reported_zero() {
    let usage = ServerToolUse {
        web_search_requests: Some(0),
        tool_search_requests: None,
        browser_open_requests: None,
    };
    assert_eq!(
        web_search_requests(Some(ServerToolRecord::Parsed(&usage))),
        Some(Json::Number(Number::from(0_i64)))
    );
}

#[test]
fn absent_record_is_absent() {
    assert_eq!(web_search_requests(None), None);
}

#[test]
fn mapping_preserves_a_reported_zero() {
    let record = mapping(&[("web_search_requests", Json::Number(Number::from(0_i64)))]);
    assert_eq!(
        web_search_requests(Some(ServerToolRecord::Mapping(&record))),
        Some(Json::Number(Number::from(0_i64)))
    );
}

#[test]
fn mapping_without_the_counter_is_absent() {
    let record = mapping(&[("tool_search_requests", Json::Number(Number::from(2_i64)))]);
    assert_eq!(
        web_search_requests(Some(ServerToolRecord::Mapping(&record))),
        None
    );
}

#[test]
fn mapping_null_counter_is_absent() {
    let record = mapping(&[("web_search_requests", Json::Null)]);
    assert_eq!(
        web_search_requests(Some(ServerToolRecord::Mapping(&record))),
        None
    );
}

#[test]
fn mapping_counter_is_returned_without_validation() {
    let record = mapping(&[("web_search_requests", Json::Text("3".to_owned()))]);
    assert_eq!(
        web_search_requests(Some(ServerToolRecord::Mapping(&record))),
        Some(Json::Text("3".to_owned()))
    );
}

#[test]
fn mapping_object_counter_is_returned_without_validation() {
    let nested = Json::Object(mapping(&[("count", Json::Number(Number::from(1_i64)))]));
    let record = mapping(&[("web_search_requests", nested.clone())]);
    assert_eq!(
        web_search_requests(Some(ServerToolRecord::Mapping(&record))),
        Some(nested)
    );
}

#[test]
fn projection_defaults_to_no_surcharge_fields() {
    let projection = ServerToolProjection::default();
    assert_eq!(projection.server_tool_use, None);
    assert_eq!(projection.inference_geo, None);
    assert_eq!(projection.speed, None);
}

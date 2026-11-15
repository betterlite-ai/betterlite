use serde_json::Value;

use super::support::{built, options};
use crate::spend_event::input::CachePresence;

fn names(event: &Value) -> Vec<&str> {
    event["request_tool_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect()
}

#[test]
fn ordinary_tools_come_first_then_passthrough_and_duplicates_stay() {
    let kwargs = r#"{
        "tools": [
            {"function": {"name": "  a  "}}, {"name": "b"}, {"function": {"x": 1}, "name": "c"},
            {"function": {"name": "   "}}, {"name": ""}, {"function": {"name": "a"}},
            {"function": "text", "name": "d"}, {"name": 5}
        ],
        "passthrough_logging_payload": {"request_body": {"tools": [{"name": "p"}, {"name": "b"}]}}
    }"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(names(&event), ["a", "b", "a", "d", "p", "b"]);
}

#[test]
fn passthrough_without_a_request_body_adds_nothing() {
    let kwargs = r#"{"tools": [{"name": "x"}], "passthrough_logging_payload": {}}"#;
    let event = built(kwargs, &options(false, CachePresence::On));
    assert_eq!(names(&event), ["x"]);
}

#[test]
fn no_tools_gives_an_empty_list() {
    let event = built("{}", &options(false, CachePresence::On));
    assert!(names(&event).is_empty());
}

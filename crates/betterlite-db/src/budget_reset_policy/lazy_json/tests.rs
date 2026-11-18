#![cfg(test)]

use super::LazyJson;
use crate::budget_reset_policy::tests::logging::{Logs, subscriber};
use serde::Serialize;
use std::collections::BTreeMap;
use tracing::subscriber::set_default;

#[derive(Serialize)]
struct Payload {
    spend: i32,
    budget: Option<&'static str>,
}

#[test]
fn lazy_json_renders_pretty_json_when_formatted() {
    let payload = Payload {
        spend: 12,
        budget: Some("1d"),
    };
    let rendered = format!("{json:?}", json = LazyJson(&payload));
    assert_eq!(rendered, "{\n  \"spend\": 12,\n  \"budget\": \"1d\"\n}");
}

#[test]
fn lazy_json_renders_inside_a_log_record() {
    let logs = Logs::default();
    let guard = set_default(subscriber(logs.clone()));
    let values = [1_i32, 2_i32];
    tracing::info!(payload = ?LazyJson(&values), "reset budget chunk");
    drop(guard);
    assert!(logs.text().contains("[\n  1,\n  2\n]"));
}

#[test]
fn lazy_json_surfaces_a_serialization_failure_to_the_formatter() {
    let failing = BTreeMap::from([((1_u8, 2_u8), 3_u8)]);
    let mut out = String::new();
    let result = std::fmt::write(
        &mut out,
        format_args!("{json:?}", json = LazyJson(&failing)),
    );
    assert!(result.is_err());
    assert!(out.is_empty());
}

#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::endpoints::passthrough_config_reads::config_read::config_endpoints;
use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use crate::endpoints::passthrough_config_reads::record::PassThroughGuardrailSettings;
use crate::endpoints::passthrough_config_reads::support::FakeConfig;
use crate::endpoints::passthrough_config_reads::support::json;

const SIMPLE: &str = r#"{"path": "/a", "target": "https://target.example.com"}"#;

#[test]
fn a_config_without_a_pass_through_list_gives_no_endpoints() {
    let read = config_endpoints(&FakeConfig::none());
    assert!(read.is_empty());
}

#[test]
fn an_empty_pass_through_list_gives_no_endpoints() {
    let read = config_endpoints(&FakeConfig::of(&[]));
    assert!(read.is_empty());
}

#[test]
fn a_config_endpoint_is_marked_as_from_the_config_file() {
    let read = config_endpoints(&FakeConfig::of(&[SIMPLE]));
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].path, "/a");
    assert_eq!(read[0].target, "https://target.example.com");
    assert!(read[0].is_from_config.0);
}

#[test]
fn a_stored_from_config_flag_is_ignored() {
    let read = config_endpoints(&FakeConfig::of(&[
        r#"{"path": "/a", "target": "https://target", "is_from_config": false}"#,
    ]));
    assert!(read[0].is_from_config.0);
}

#[test]
fn a_config_endpoint_gets_a_generated_id() {
    let source = FakeConfig::of(&[SIMPLE]);
    let first = config_endpoints(&source);
    let second = config_endpoints(&source);
    let first_id = first[0].id.as_deref().expect("a generated id");
    assert!(Uuid::parse_str(first_id).is_ok(), "{first_id}");
    assert_ne!(first[0].id, second[0].id);
}

#[test]
fn a_config_endpoint_keeps_the_id_it_declares() {
    let read = config_endpoints(&FakeConfig::of(&[
        r#"{"id": "declared", "path": "/a", "target": "https://target"}"#,
    ]));
    assert_eq!(read[0].id.as_deref(), Some("declared"));
}

#[test]
fn a_config_endpoint_without_defaults_takes_every_default() {
    let read = config_endpoints(&FakeConfig::of(&[SIMPLE]));
    let endpoint = &read[0];
    assert!(endpoint.headers.is_empty());
    assert!(endpoint.default_query_params.is_empty());
    assert!(!endpoint.include_subpath.0);
    assert_eq!(endpoint.cost_per_request, Decimal::ZERO);
    assert_eq!(endpoint.timeout, None);
    assert!(endpoint.auth.0);
    assert_eq!(endpoint.guardrails, None);
    assert_eq!(endpoint.methods, None);
}

#[rstest]
#[case::headers(r#""headers": null"#)]
#[case::query_params(r#""default_query_params": null"#)]
#[case::include_subpath(r#""include_subpath": null"#)]
#[case::cost_per_request(r#""cost_per_request": null"#)]
#[case::timeout(r#""timeout": null"#)]
#[case::auth(r#""auth": null"#)]
#[case::guardrails(r#""guardrails": null"#)]
#[case::methods(r#""methods": null"#)]
#[case::id(r#""id": null"#)]
fn a_null_config_value_takes_its_default(#[case] defaulted: &str) {
    let entry = format!(r#"{{"path": "/a", "target": "https://target", {defaulted}}}"#);
    let read = config_endpoints(&FakeConfig::of(&[entry.as_str()]));
    let endpoint = &read[0];
    assert!(
        endpoint.id.is_some(),
        "a null id still gets an id at read time"
    );
    assert!(endpoint.headers.is_empty());
    assert!(endpoint.default_query_params.is_empty());
    assert!(!endpoint.include_subpath.0);
    assert_eq!(endpoint.cost_per_request, Decimal::ZERO);
    assert_eq!(endpoint.timeout, None);
    assert!(endpoint.auth.0);
    assert_eq!(endpoint.guardrails, None);
    assert_eq!(endpoint.methods, None);
}

#[test]
fn the_declared_config_values_are_read() {
    let read = config_endpoints(&FakeConfig::of(&[
        r#"{"id": "c1", "path": "/a", "target": "https://target", "headers": {"x-key": "secret"}, "default_query_params": {"region": "eu"}, "include_subpath": true, "cost_per_request": 0.25, "timeout": 5, "auth": false, "guardrails": {"pii": {"request_fields": ["prompt"], "response_fields": ["text"]}}, "methods": ["GET", "POST"]}"#,
    ]));
    let endpoint = &read[0];
    assert_eq!(endpoint.headers.get("x-key"), Some(&json(r#""secret""#)));
    assert_eq!(
        endpoint.default_query_params.get("region"),
        Some(&json(r#""eu""#))
    );
    assert!(endpoint.include_subpath.0);
    assert_eq!(endpoint.cost_per_request, Decimal::new(25, 2));
    assert_eq!(endpoint.timeout.map(f64::to_bits), Some(5.0_f64.to_bits()));
    assert!(!endpoint.auth.0);
    let guardrails = BTreeMap::from([(
        "pii".to_owned(),
        Some(PassThroughGuardrailSettings {
            request_fields: Some(vec!["prompt".to_owned()]),
            response_fields: Some(vec!["text".to_owned()]),
        }),
    )]);
    assert_eq!(endpoint.guardrails, Some(guardrails));
    assert_eq!(
        endpoint.methods,
        Some(vec!["GET".to_owned(), "POST".to_owned()])
    );
}

#[test]
fn guardrail_settings_keep_their_null_defaults() {
    let read = config_endpoints(&FakeConfig::of(&[
        r#"{"path": "/a", "target": "https://target", "guardrails": {"pii": null}}"#,
        r#"{"path": "/b", "target": "https://target", "guardrails": {"pii": {}}}"#,
    ]));
    assert_eq!(read.len(), 2);
    assert_eq!(
        read[0].guardrails,
        Some(BTreeMap::from([("pii".to_owned(), None)]))
    );
    assert_eq!(
        read[1].guardrails,
        Some(BTreeMap::from([(
            "pii".to_owned(),
            Some(PassThroughGuardrailSettings {
                request_fields: None,
                response_fields: None,
            }),
        )]))
    );
}

#[test]
fn an_unknown_config_key_is_ignored() {
    let read = config_endpoints(&FakeConfig::of(&[
        r#"{"path": "/a", "target": "https://target", "unknown": true}"#,
    ]));
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].path, "/a");
}

#[rstest]
#[case::missing_target(r#"{"path": "/broken"}"#)]
#[case::missing_path(r#"{"target": "https://target"}"#)]
#[case::wrong_typed_flag(
    r#"{"path": "/broken", "target": "https://target", "include_subpath": "yes"}"#
)]
#[case::non_object_entry(r#""text""#)]
#[case::numeric_entry("7")]
fn a_malformed_config_endpoint_is_skipped(#[case] broken: &str) {
    let source = FakeConfig::of(&[broken, SIMPLE]);
    let read = config_endpoints(&source);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].path, "/a");
}

fn replay_corpus() -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/pass_through_config_reads");
    let mut replayed = 0_usize;
    for entry in fs::read_dir(&root).expect("the corpus directory") {
        let path = entry.expect("a corpus entry").path();
        let Ok(text) = fs::read_to_string(path) else {
            replayed = replayed.saturating_add(1);
            continue;
        };
        let Ok(value) = serde_json::from_str::<JsonData>(&text) else {
            replayed = replayed.saturating_add(1);
            continue;
        };
        drop(PassThroughGenericEndpoint::from_json(&value, true));
        let source = FakeConfig {
            entries: Some(vec![value]),
        };
        drop(config_endpoints(&source));
        replayed = replayed.saturating_add(1);
    }
    replayed
}

#[test]
fn replays_the_pass_through_config_reads_corpus() {
    assert!(replay_corpus() > 0);
}

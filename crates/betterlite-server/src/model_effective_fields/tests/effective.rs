use rstest::rstest;

use crate::model_effective_fields::decrypted_model;
use crate::model_effective_fields::effective::{effective_complexity_config, effective_model};

use super::support::{cipher, json, object, sealed};

#[test]
fn effective_model_prefers_the_incoming_model() {
    let stored = format!(r#"{{"model": "{}"}}"#, sealed("gpt-4o"));
    let model = effective_model(
        &cipher(),
        Some(&object(r#"{"model": "claude"}"#)),
        Some(&object(&stored)),
    );
    assert_eq!(model.as_deref(), Some("claude"));
}

#[rstest]
#[case::omitted("{}")]
#[case::null(r#"{"model": null}"#)]
fn effective_model_decrypts_the_stored_model(#[case] incoming: &str) {
    let stored = format!(r#"{{"model": "{}"}}"#, sealed("gpt-4o"));
    let model = effective_model(&cipher(), Some(&object(incoming)), Some(&object(&stored)));
    assert_eq!(model.as_deref(), Some("gpt-4o"));
}

#[test]
fn effective_model_keeps_a_stored_model_that_does_not_open() {
    let model = effective_model(&cipher(), None, Some(&object(r#"{"model": "gpt-4o"}"#)));
    assert_eq!(model.as_deref(), Some("gpt-4o"));
}

#[rstest]
#[case::no_row(None)]
#[case::no_model(Some("{}"))]
#[case::not_text(Some(r#"{"model": 7}"#))]
fn effective_model_is_absent_without_a_text_model(#[case] existing: Option<&str>) {
    let existing = existing.map(object);
    assert_eq!(effective_model(&cipher(), None, existing.as_ref()), None);
}

#[test]
fn decrypted_model_reads_only_text() {
    assert_eq!(decrypted_model(&cipher(), Some(&json("[1]"))), None);
    let stored = json(&format!(r#""{}""#, sealed("auto_router/x")));
    assert_eq!(
        decrypted_model(&cipher(), Some(&stored)).as_deref(),
        Some("auto_router/x")
    );
}

const STORED_JEV: &str = r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"api_key": "k", "api_base": "b", "model": "m1"}}}"#;

#[rstest]
#[case::nothing_sent("{}", STORED_JEV, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"api_key": "k", "api_base": "b", "model": "m1"}}"#))]
#[case::null_sent(r#"{"complexity_router_config": null}"#, "{}", None)]
#[case::nothing_stored(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"model": "m2"}}}"#, "{}", Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"model": "m2"}}"#))]
#[case::same_base_keeps_the_key(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"model": "m2"}}}"#, STORED_JEV, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"api_key": "k", "api_base": "b", "model": "m2"}}"#))]
#[case::repeated_base_keeps_the_key(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"api_base": "b"}}}"#, STORED_JEV, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"api_key": "k", "api_base": "b"}}"#))]
#[case::new_base_drops_the_key(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"api_base": "other"}}}"#, STORED_JEV, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"api_base": "other"}}"#))]
#[case::null_base_drops_a_stored_key(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"api_base": null}}}"#, STORED_JEV, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"api_base": null}}"#))]
#[case::null_base_matches_no_stored_base(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"api_base": null}}}"#, r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"api_key": "k"}}}"#, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"api_key": "k", "api_base": null}}"#))]
#[case::other_keys_stay_behind(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {}}}"#, r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"model": "m1", "api_base": "b"}}}"#, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"api_base": "b"}}"#))]
#[case::incoming_not_jev(
    r#"{"complexity_router_config": {"classifier_type": "heuristic"}}"#,
    STORED_JEV,
    Some(r#"{"classifier_type": "heuristic"}"#)
)]
#[case::stored_not_jev(r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {"model": "m2"}}}"#, r#"{"complexity_router_config": {"classifier_type": "heuristic", "jev_classifier_config": {"api_key": "k"}}}"#, Some(r#"{"classifier_type": "jev", "jev_classifier_config": {"model": "m2"}}"#))]
#[case::jev_config_not_an_object(
    r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": "m2"}}"#,
    STORED_JEV,
    Some(r#"{"classifier_type": "jev", "jev_classifier_config": "m2"}"#)
)]
#[case::stored_jev_config_not_an_object(
    r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": {}}}"#,
    r#"{"complexity_router_config": {"classifier_type": "jev", "jev_classifier_config": null}}"#,
    Some(r#"{"classifier_type": "jev", "jev_classifier_config": {}}"#)
)]
#[case::config_not_an_object(
    r#"{"complexity_router_config": "jev"}"#,
    STORED_JEV,
    Some(r#""jev""#)
)]
fn jev_transport_merge(
    #[case] incoming: &str,
    #[case] existing: &str,
    #[case] expected: Option<&str>,
) {
    let merged = effective_complexity_config(Some(&object(incoming)), Some(&object(existing)));
    assert_eq!(merged, expected.map(json));
    let encoded = serde_json::to_string(&merged).unwrap();
    assert_eq!(encoded, serde_json::to_string(&expected.map(json)).unwrap());
}

#[test]
fn effective_config_without_any_params_is_absent() {
    assert_eq!(effective_complexity_config(None, None), None);
}

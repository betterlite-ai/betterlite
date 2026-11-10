#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::request_labels::{pop_vertex_request_labels, request_labels_from_params};

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn labels_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::explicit_labels_win(
    Some(r#"{"labels": {"team": "core", "count": 3}, "temperature": 0.2}"#),
    Some(r#"{"metadata": {"requester_metadata": {"team": "fallback"}}}"#),
    Some(&[("team", "core")][..]),
    Some(r#"{"temperature": 0.2}"#),
    Some(&[("team", "fallback")][..])
)]
#[case::non_string_labels_fall_through(
    Some(r#"{"labels": {"count": 3, "ratio": 0.5}}"#),
    Some(r#"{"metadata": {"requester_metadata": {"team": "from_metadata"}}}"#),
    Some(&[("team", "from_metadata")][..]),
    Some("{}"),
    Some(&[("team", "from_metadata")][..])
)]
#[case::empty_labels_fall_through(
    Some(r#"{"labels": {}}"#),
    Some(r#"{"metadata": {"requester_metadata": {"team": "from_metadata"}}}"#),
    Some(&[("team", "from_metadata")][..]),
    Some("{}"),
    Some(&[("team", "from_metadata")][..])
)]
#[case::labels_absent_use_metadata(
    Some(r#"{"temperature": 0.2}"#),
    Some(r#"{"metadata": {"requester_metadata": {"team": "from_metadata"}}}"#),
    Some(&[("team", "from_metadata")][..]),
    Some(r#"{"temperature": 0.2}"#),
    Some(&[("team", "from_metadata")][..])
)]
#[case::metadata_falls_to_litellm_metadata(
    None,
    Some(r#"{"metadata": {"requester_metadata": {"count": 2}}, "litellm_metadata": {"requester_metadata": {"team": "second"}}}"#),
    Some(&[("team", "second")][..]),
    None,
    Some(&[("team", "second")][..])
)]
#[case::null_metadata_falls_through(
    None,
    Some(r#"{"metadata": null, "litellm_metadata": {"requester_metadata": {"team": "second"}}}"#),
    Some(&[("team", "second")][..]),
    None,
    Some(&[("team", "second")][..])
)]
#[case::requester_not_a_map(
    None,
    Some(r#"{"metadata": {"requester_metadata": "team"}, "litellm_metadata": {"requester_metadata": {"team": "second"}}}"#),
    Some(&[("team", "second")][..]),
    None,
    Some(&[("team", "second")][..])
)]
#[case::metadata_and_litellm_metadata_use_metadata(
    None,
    Some(r#"{"metadata": {"requester_metadata": {"team": "first"}}, "litellm_metadata": {"requester_metadata": {"team": "second"}}}"#),
    Some(&[("team", "first")][..]),
    None,
    Some(&[("team", "first")][..])
)]
#[case::null_labels_are_popped(
    Some(r#"{"labels": null, "temperature": 1}"#),
    Some(r#"{"metadata": {"requester_metadata": {"team": "from_metadata"}}}"#),
    Some(&[("team", "from_metadata")][..]),
    Some(r#"{"temperature": 1}"#),
    Some(&[("team", "from_metadata")][..])
)]
#[case::non_map_labels_are_popped(Some(r#"{"labels": ["team"]}"#), None, None, Some("{}"), None)]
#[case::only_string_values_survive(
    Some(r#"{"labels": {"team": "core", "count": 3, "active": true}}"#),
    None,
    Some(&[("team", "core")][..]),
    Some("{}"),
    None
)]
#[case::no_params(None, None, None, None, None)]
#[case::empty_params(None, Some("{}"), None, None, None)]
#[case::empty_requester_map(
    None,
    Some(r#"{"litellm_metadata": {"requester_metadata": {}}}"#),
    None,
    None,
    None
)]
fn label_cases(
    #[case] optional_params: Option<&str>,
    #[case] litellm_params: Option<&str>,
    #[case] expected_labels: Option<&[(&str, &str)]>,
    #[case] expected_optional_params: Option<&str>,
    #[case] expected_from_params: Option<&[(&str, &str)]>,
) {
    let mut optional_value = optional_params.map(json);
    let params_value = litellm_params.map(json);
    let labels = pop_vertex_request_labels(&mut optional_value, params_value.as_ref());
    assert_eq!(labels, expected_labels.map(labels_map));
    assert_eq!(optional_value, expected_optional_params.map(json));
    let from_params = request_labels_from_params(params_value.as_ref());
    assert_eq!(from_params, expected_from_params.map(labels_map));
}

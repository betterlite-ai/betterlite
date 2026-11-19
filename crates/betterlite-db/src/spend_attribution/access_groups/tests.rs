#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use crate::spend_attribution::support::FixtureDeployments;

use super::{debitable_model_access_groups, served_model_access_groups};

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn a_router_without_the_deployment_serves_no_narrowing() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["a"]}"#)]);
    assert_eq!(
        served_model_access_groups(Some(&deployments), Some("dep-2")),
        None
    );
}

#[test]
fn no_router_or_no_served_id_serves_no_narrowing() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["a"]}"#)]);
    assert_eq!(served_model_access_groups(None, Some("dep-1")), None);
    assert_eq!(served_model_access_groups(Some(&deployments), None), None);
    assert_eq!(
        served_model_access_groups(Some(&deployments), Some("")),
        None
    );
}

#[test]
fn a_known_deployment_reads_its_declared_groups() {
    let deployments =
        FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["b","a","b",1,null,["c"]]}"#)]);
    assert_eq!(
        served_model_access_groups(Some(&deployments), Some("dep-1")),
        Some(set(&["a", "b"]))
    );
}

#[rstest]
#[case::no_group_list("dep-1")]
#[case::text_groups("dep-2")]
#[case::empty_group_list("dep-3")]
fn a_known_deployment_without_a_group_list_serves_an_empty_set(#[case] deployment_id: &str) {
    let deployments = FixtureDeployments::new(&[
        ("dep-1", r#"{"id":"dep-1"}"#),
        ("dep-2", r#"{"access_groups":"a"}"#),
        ("dep-3", r#"{"access_groups":[]}"#),
    ]);
    assert_eq!(
        served_model_access_groups(Some(&deployments), Some(deployment_id)),
        Some(BTreeSet::new())
    );
}

#[test]
fn without_a_served_deployment_every_attributed_group_debits() {
    let attributed = names(&["b", "a", "", "b", "c"]);
    assert_eq!(
        debitable_model_access_groups(&attributed, None, None),
        names(&["b", "a", "c"])
    );
}

#[test]
fn an_unknown_served_deployment_keeps_the_attributed_order() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["a"]}"#)]);
    let attributed = names(&["c", "a"]);
    assert_eq!(
        debitable_model_access_groups(&attributed, Some("dep-2"), Some(&deployments)),
        names(&["c", "a"])
    );
}

#[test]
fn a_served_deployment_only_narrows_the_attributed_groups() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["b","d","a"]}"#)]);
    let attributed = names(&["c", "b", "a", "b"]);
    assert_eq!(
        debitable_model_access_groups(&attributed, Some("dep-1"), Some(&deployments)),
        names(&["b", "a"])
    );
}

#[test]
fn a_served_deployment_cannot_add_a_group() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["d"]}"#)]);
    let attributed = names(&["a"]);
    assert!(
        debitable_model_access_groups(&attributed, Some("dep-1"), Some(&deployments)).is_empty()
    );
}

#[test]
fn a_deployment_without_groups_debits_nothing() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"id":"dep-1"}"#)]);
    let attributed = names(&["a", "b"]);
    assert!(
        debitable_model_access_groups(&attributed, Some("dep-1"), Some(&deployments)).is_empty()
    );
}

#[test]
fn the_order_comes_from_the_attribution_not_the_deployment() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["a","b","c"]}"#)]);
    let attributed = names(&["c", "a", "b"]);
    assert_eq!(
        debitable_model_access_groups(&attributed, Some("dep-1"), Some(&deployments)),
        names(&["c", "a", "b"])
    );
}

#[test]
fn an_empty_served_id_reads_no_narrowing() {
    let deployments = FixtureDeployments::new(&[("", r#"{"access_groups":["a"]}"#)]);
    assert_eq!(
        served_model_access_groups(Some(&deployments), Some("")),
        None
    );
}

#[test]
fn nothing_attributed_debits_nothing() {
    let deployments = FixtureDeployments::new(&[("dep-1", r#"{"access_groups":["a"]}"#)]);
    let attributed: Vec<String> = Vec::new();
    assert!(debitable_model_access_groups(&attributed, None, Some(&deployments)).is_empty());
    let empty_names = names(&["", ""]);
    assert!(debitable_model_access_groups(&empty_names, None, None).is_empty());
}

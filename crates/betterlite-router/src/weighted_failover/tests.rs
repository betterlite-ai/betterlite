#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value};

use super::{FailoverEntry, FailoverInputs, drop_excluded, plan_weighted_failover};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn keyed(metadata: Value, key: &str) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert(key.to_owned(), metadata);
    object
}

fn plan(inputs: &FailoverInputs<'_>, request: &mut Map<String, Value>) -> Option<FailoverEntry> {
    plan_weighted_failover(inputs, request, "metadata")
}

fn inputs<'input>(
    failed: Option<&'input str>,
    deployments: &'input [String],
) -> FailoverInputs<'input> {
    FailoverInputs {
        strategy: "simple-shuffle",
        failed_deployment_id: failed,
        model_group: "gpt-4o",
        deployment_ids: deployments,
        cooldown_ids: &[],
    }
}

#[rstest]
#[case::latency("latency-based-routing")]
#[case::usage("usage-based-routing")]
#[case::cost("cost-based-routing")]
#[case::least_busy("least-busy")]
fn plan_needs_the_simple_shuffle_strategy(#[case] strategy: &str) {
    let deployments = ids(&["a", "b"]);
    let mut request = keyed(value("{}"), "metadata");
    let failing = FailoverInputs {
        strategy,
        failed_deployment_id: Some("a"),
        model_group: "gpt-4o",
        deployment_ids: &deployments,
        cooldown_ids: &[],
    };
    assert_eq!(plan(&failing, &mut request), None);
    assert_eq!(request["metadata"], value("{}"));
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn plan_needs_a_failed_deployment_id(#[case] failed: Option<&str>) {
    let deployments = ids(&["a", "b"]);
    let mut request = Map::new();
    assert_eq!(plan(&inputs(failed, &deployments), &mut request), None);
    assert_eq!(request, Map::new());
}

#[rstest]
#[case::absent(Map::new())]
#[case::null(keyed(value("null"), "metadata"))]
fn plan_inserts_an_empty_metadata_object(#[case] mut request: Map<String, Value>) {
    let deployments = ids(&["a"]);
    assert_eq!(plan(&inputs(Some("a"), &deployments), &mut request), None);
    assert_eq!(request["metadata"], value("{}"));
}

#[rstest]
#[case::text(value("\"not-an-object\""))]
#[case::number(value("7"))]
#[case::array(value("[]"))]
fn plan_gives_none_when_metadata_is_not_an_object(#[case] metadata: Value) {
    let deployments = ids(&["a", "b"]);
    let mut request = keyed(metadata.clone(), "metadata");
    assert_eq!(plan(&inputs(Some("a"), &deployments), &mut request), None);
    assert_eq!(request["metadata"], metadata);
}

#[test]
fn plan_adds_the_failed_id_to_earlier_exclusions() {
    let deployments = ids(&["a", "b", "c"]);
    let mut request = keyed(value("{\"_failover_excluded_ids\": [\"a\"]}"), "metadata");
    let entry = plan(&inputs(Some("b"), &deployments), &mut request).unwrap();
    assert_eq!(entry.excluded_deployment_ids, ids(&["a", "b"]));
    assert_eq!(
        request["metadata"]["_failover_excluded_ids"],
        value("[\"a\", \"b\"]")
    );
}

#[test]
fn plan_keeps_each_exclusion_once_in_first_seen_order() {
    let deployments = ids(&["a", "b", "c"]);
    let mut request = keyed(
        value("{\"_failover_excluded_ids\": [\"b\", \"a\", \"b\"]}"),
        "metadata",
    );
    let entry = plan(&inputs(Some("b"), &deployments), &mut request).unwrap();
    assert_eq!(entry.excluded_deployment_ids, ids(&["b", "a"]));
}

#[test]
fn plan_counts_a_cooling_deployment_as_unavailable() {
    let deployments = ids(&["a", "b", "c"]);
    let cooling = ids(&["b", "c"]);
    let mut request = Map::new();
    let failing = FailoverInputs {
        strategy: "simple-shuffle",
        failed_deployment_id: Some("a"),
        model_group: "gpt-4o",
        deployment_ids: &deployments,
        cooldown_ids: &cooling,
    };
    assert_eq!(plan(&failing, &mut request), None);
    assert!(request["metadata"].get("_failover_excluded_ids").is_none());
}

#[rstest]
#[case::null("null", &["b"])]
#[case::bare_string("\"a\"", &["b"])]
#[case::non_string_entry("[\"a\", 1]", &["a", "b"])]
fn plan_reads_earlier_exclusions_leniently(#[case] earlier: &str, #[case] expected: &[&str]) {
    let deployments = ids(&["a", "b", "c"]);
    let mut request = keyed(
        value(&format!("{{\"_failover_excluded_ids\": {earlier}}}")),
        "metadata",
    );
    let entry = plan(&inputs(Some("b"), &deployments), &mut request).unwrap();
    assert_eq!(entry.excluded_deployment_ids, ids(expected));
}

#[rstest]
#[case::only_deployment(&["a"], &[])]
#[case::remaining_is_cooling(&["a", "b"], &["b"])]
fn plan_gives_none_when_no_deployment_remains(
    #[case] deployment_names: &[&str],
    #[case] cooling_names: &[&str],
) {
    let deployments = ids(deployment_names);
    let cooling = ids(cooling_names);
    let mut request = Map::new();
    let failing = FailoverInputs {
        strategy: "simple-shuffle",
        failed_deployment_id: Some("a"),
        model_group: "gpt-4o",
        deployment_ids: &deployments,
        cooldown_ids: &cooling,
    };
    assert_eq!(plan(&failing, &mut request), None);
    assert_eq!(request["metadata"], value("{}"));
    assert!(request["metadata"].get("_failover_excluded_ids").is_none());
}

#[rstest]
#[case::metadata("metadata")]
#[case::litellm_metadata("litellm_metadata")]
fn plan_writes_the_exclusions_back_under_the_metadata_key(#[case] metadata_key: &str) {
    let deployments = ids(&["a", "b"]);
    let mut request = keyed(value("{}"), metadata_key);
    let entry =
        plan_weighted_failover(&inputs(Some("a"), &deployments), &mut request, metadata_key);
    assert_eq!(
        entry.map(|planned| planned.excluded_deployment_ids),
        Some(ids(&["a"]))
    );
    assert_eq!(
        request[metadata_key]["_failover_excluded_ids"],
        value("[\"a\"]")
    );
}

#[test]
fn plan_returns_an_entry_that_serializes_as_model_and_excluded_ids() {
    let deployments = ids(&["a", "b"]);
    let mut request = Map::new();
    let entry = plan(&inputs(Some("a"), &deployments), &mut request).unwrap();
    assert_eq!(
        serde_json::to_value(entry).unwrap(),
        value("{\"model\": \"gpt-4o\", \"_excluded_deployment_ids\": [\"a\"]}")
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Deployment {
    name: String,
    model_info_id: Option<String>,
}

fn deployments(entries: &[(&str, Option<&str>)]) -> Vec<Deployment> {
    entries
        .iter()
        .map(|(name, id)| Deployment {
            name: (*name).to_owned(),
            model_info_id: id.map(String::from),
        })
        .collect()
}

fn model_info_id(deployment: &Deployment) -> Option<&str> {
    deployment.model_info_id.as_deref()
}

fn names(deployments: &[Deployment]) -> Vec<&str> {
    deployments
        .iter()
        .map(|deployment| deployment.name.as_str())
        .collect()
}

#[test]
fn excluded_returns_the_input_for_an_empty_list() {
    let candidates = deployments(&[("a", Some("a")), ("b", Some("b"))]);
    let kept = drop_excluded(candidates.clone(), &[], model_info_id);
    assert_eq!(kept, candidates);
}

#[test]
fn excluded_keeps_order_and_deployments_without_an_id() {
    let candidates = deployments(&[
        ("first", None),
        ("second", Some("a")),
        ("third", Some("b")),
        ("fourth", Some("c")),
    ]);
    let kept = drop_excluded(candidates, &ids(&["b"]), model_info_id);
    assert_eq!(names(&kept), vec!["first", "second", "fourth"]);
}

#[test]
fn excluded_may_return_an_empty_list() {
    let candidates = deployments(&[("a", Some("a")), ("b", None)]);
    let kept = drop_excluded(candidates, &ids(&["a"]), model_info_id);
    assert_eq!(names(&kept), vec!["b"]);
    let everything = deployments(&[("a", Some("a"))]);
    assert!(drop_excluded(everything, &ids(&["a"]), model_info_id).is_empty());
}

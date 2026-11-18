use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use crate::prometheus::failure::model_cardinality::bounded_requested_model_label;
use crate::prometheus::failure::tests::support::StubCatalog;

#[test]
fn an_absent_model_is_no_label() {
    assert_eq!(bounded_requested_model_label(None, false, None), None);
}

#[test]
fn a_json_null_model_is_no_label() {
    assert_eq!(
        bounded_requested_model_label(Some(&Json::Null), false, None),
        None
    );
}

#[test]
fn an_empty_model_stays_empty() {
    let model = Json::Text(String::new());
    assert_eq!(
        bounded_requested_model_label(Some(&model), false, None),
        Some(String::new())
    );
}

#[rstest]
#[case::boolean(Json::Bool(true))]
#[case::number(Json::Number(Number::from(3)))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(IndexMap::new()))]
fn non_string_collapse(#[case] model: Json) {
    assert_eq!(
        bounded_requested_model_label(Some(&model), true, None),
        Some(String::from("other"))
    );
}

#[test]
fn a_recognized_model_keeps_its_name() {
    let catalog = StubCatalog {
        recognized: BTreeSet::from([String::from("gpt-4o")]),
        team_public: BTreeSet::new(),
        patterns: BTreeSet::new(),
    };
    let model = Json::Text(String::from("gpt-4o"));
    assert_eq!(
        bounded_requested_model_label(Some(&model), false, Some(&catalog)),
        Some(String::from("gpt-4o"))
    );
}

#[test]
fn a_team_public_model_keeps_its_name() {
    let catalog = StubCatalog {
        recognized: BTreeSet::new(),
        team_public: BTreeSet::from([String::from("shared-model")]),
        patterns: BTreeSet::new(),
    };
    let model = Json::Text(String::from("shared-model"));
    assert_eq!(
        bounded_requested_model_label(Some(&model), false, Some(&catalog)),
        Some(String::from("shared-model"))
    );
}

#[test]
fn a_pattern_route_match_keeps_its_name() {
    let catalog = StubCatalog {
        recognized: BTreeSet::new(),
        team_public: BTreeSet::new(),
        patterns: BTreeSet::from([String::from("gpt-4-*")]),
    };
    let model = Json::Text(String::from("gpt-4-*"));
    assert_eq!(
        bounded_requested_model_label(Some(&model), false, Some(&catalog)),
        Some(String::from("gpt-4-*"))
    );
}

#[test]
fn an_unrecognized_model_collapses() {
    let catalog = StubCatalog {
        recognized: BTreeSet::new(),
        team_public: BTreeSet::new(),
        patterns: BTreeSet::new(),
    };
    let model = Json::Text(String::from("client-invented"));
    assert_eq!(
        bounded_requested_model_label(Some(&model), false, Some(&catalog)),
        Some(String::from("other"))
    );
}

#[test]
fn a_router_originated_name_still_collapses_when_no_router_knows_it() {
    let catalog = StubCatalog {
        recognized: BTreeSet::new(),
        team_public: BTreeSet::new(),
        patterns: BTreeSet::new(),
    };
    let model = Json::Text(String::from("client-invented"));
    assert_eq!(
        bounded_requested_model_label(Some(&model), true, Some(&catalog)),
        Some(String::from("other"))
    );
}

#[test]
fn a_client_name_collapses_without_a_router() {
    let model = Json::Text(String::from("gpt-4o"));
    assert_eq!(
        bounded_requested_model_label(Some(&model), false, None),
        Some(String::from("other"))
    );
}

#[test]
fn a_router_originated_name_passes_without_a_router() {
    let model = Json::Text(String::from("gpt-4o"));
    assert_eq!(
        bounded_requested_model_label(Some(&model), true, None),
        Some(String::from("gpt-4o"))
    );
}

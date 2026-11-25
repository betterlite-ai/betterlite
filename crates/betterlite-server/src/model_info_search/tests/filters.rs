use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::support::{list, model, number, object, params, text};
use crate::model_info_search::filters::{
    is_auto_router_model, matches_model_info_filters, model_in_access_group,
};

fn with_params(name: &str, backend: Json) -> JsonObject {
    object(vec![
        ("model_name", text(name)),
        (
            "litellm_params",
            Json::Object(object(vec![("model", backend)])),
        ),
        ("model_info", Json::Object(object(Vec::new()))),
    ])
}

fn with_groups(name: &str, groups: Json) -> JsonObject {
    model(name, vec![("access_groups", groups)])
}

#[rstest]
#[case::semantic("auto_router/semantic")]
#[case::complexity("auto_router/complexity")]
#[case::adaptive("auto_router/adaptive")]
#[case::quality("auto_router/quality")]
fn every_auto_router_strategy_matches(#[case] backend: &str) {
    let deployment = with_params("auto", text(backend));
    assert!(is_auto_router_model(&deployment));
    assert!(!matches_model_info_filters(
        &deployment,
        Some(true),
        None,
        None
    ));
    assert!(matches_model_info_filters(
        &deployment,
        Some(false),
        None,
        None
    ));
    assert!(matches_model_info_filters(&deployment, None, None, None));
}

#[test]
fn only_a_string_backend_prefix_matches_auto_router() {
    assert!(!is_auto_router_model(&with_params(
        "close",
        text("auto_router")
    )));
    assert!(!is_auto_router_model(&with_params(
        "nested",
        text("vendor/auto_router/semantic")
    )));
    assert!(!is_auto_router_model(&with_params("number", number(1))));
    assert!(!is_auto_router_model(&object(vec![
        ("model_name", text("no-params")),
        ("litellm_params", text("not an object")),
    ])));
}

#[test]
fn access_group_filter() {
    let deployment = with_groups("g", list(vec![text("team-a"), text("team b")]));
    assert!(model_in_access_group(&deployment, "team-a"));
    assert!(model_in_access_group(&deployment, "team b"));
    assert!(!model_in_access_group(&deployment, "team"));
    assert!(!model_in_access_group(&deployment, "TEAM-A"));
    assert!(!model_in_access_group(&deployment, ""));

    let empty = with_groups("empty", list(vec![text("")]));
    assert!(model_in_access_group(&empty, ""));
    assert!(matches_model_info_filters(&empty, None, Some(""), None));
    assert!(!matches_model_info_filters(
        &empty,
        None,
        Some("team-a"),
        None
    ));
}

#[rstest]
#[case::missing(model("missing", Vec::new()))]
#[case::text(with_groups("text", text("team-a")))]
#[case::object(with_groups("object", Json::Object(object(Vec::new()))))]
#[case::number(with_groups("number", number(1)))]
#[case::foreign_items(with_groups(
    "foreign",
    list(vec![number(1), Json::Object(object(Vec::new()))])
))]
fn malformed_access_group_metadata_does_not_match(#[case] deployment: JsonObject) {
    assert!(!model_in_access_group(&deployment, "1"));
    assert!(!matches_model_info_filters(
        &deployment,
        None,
        Some("1"),
        None
    ));
}

#[test]
fn wildcard_filter_needs_a_literal_true() {
    let star = model("*", Vec::new());
    let prefixed = model("gpt-*", Vec::new());
    let plain = model("gpt-4o", Vec::new());
    assert!(matches_model_info_filters(&star, None, None, Some(true)));
    assert!(matches_model_info_filters(
        &prefixed,
        None,
        None,
        Some(true)
    ));
    assert!(!matches_model_info_filters(&plain, None, None, Some(true)));
    assert!(matches_model_info_filters(&plain, None, None, Some(false)));
    assert!(matches_model_info_filters(&plain, None, None, None));
}

#[test]
fn filters_apply_together() {
    let deployment = object(vec![
        ("model_name", text("team-*")),
        (
            "litellm_params",
            Json::Object(object(vec![("model", text("openai/gpt-4o"))])),
        ),
        (
            "model_info",
            Json::Object(object(vec![("access_groups", list(vec![text("team-a")]))])),
        ),
    ]);
    assert!(matches_model_info_filters(
        &deployment,
        Some(true),
        Some("team-a"),
        Some(true)
    ));
    assert!(!matches_model_info_filters(
        &deployment,
        Some(true),
        Some("team-b"),
        Some(true)
    ));
    assert!(!matches_model_info_filters(
        &deployment,
        Some(true),
        Some("team-a"),
        Some(false)
    ));
    let router = with_params("team-*", text("auto_router/semantic"));
    assert!(!matches_model_info_filters(
        &router,
        Some(true),
        None,
        Some(true)
    ));
}

#[test]
fn a_missing_model_name_never_matches_a_wildcard() {
    let deployment = object(vec![
        ("litellm_params", params("openai/gpt-4o")),
        ("model_info", Json::Object(object(Vec::new()))),
    ]);
    assert!(!matches_model_info_filters(
        &deployment,
        None,
        None,
        Some(true)
    ));
}

use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use crate::health_access_and_connections::result_filter::{
    filter_health_check_results_by_model_ids, strip_admin_only_fields,
};

use super::support::{object, text};

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn rows(entry: &JsonObject, key: &str) -> Vec<Json> {
    match entry.get(key) {
        Some(Json::List(rows)) => rows.clone(),
        other => panic!("{key} is a list, got {other:?}"),
    }
}

fn result_with_routing_fields() -> JsonObject {
    object(vec![
        (
            "healthy_endpoints",
            Json::List(vec![Json::Object(object(vec![
                ("model_id", text("d1")),
                ("model", text("gpt")),
                ("api_base", text("https://config")),
                ("api_version", text("2024-01-01")),
                ("aws_bedrock_runtime_endpoint", text("https://bedrock")),
            ]))]),
        ),
        (
            "unhealthy_endpoints",
            Json::List(vec![Json::Object(object(vec![
                ("model_id", text("d2")),
                ("api_base", text("https://config")),
            ]))]),
        ),
        ("healthy_count", Json::Number(Number::from(1_u64))),
    ])
}

#[test]
fn stripping_removes_the_routing_fields_from_each_object_row() {
    let stripped = strip_admin_only_fields(&result_with_routing_fields());
    assert_eq!(
        rows(&stripped, "healthy_endpoints"),
        vec![Json::Object(object(vec![
            ("model_id", text("d1")),
            ("model", text("gpt")),
        ]))]
    );
    assert_eq!(
        rows(&stripped, "unhealthy_endpoints"),
        vec![Json::Object(object(vec![("model_id", text("d2"))]))]
    );
    assert_eq!(
        stripped.get("healthy_count"),
        Some(&Json::Number(Number::from(1_u64)))
    );
}

#[test]
fn stripping_leaves_the_source_rows_untouched() {
    let result = result_with_routing_fields();
    let stripped = strip_admin_only_fields(&result);
    assert_eq!(
        rows(&result, "healthy_endpoints"),
        vec![Json::Object(object(vec![
            ("model_id", text("d1")),
            ("model", text("gpt")),
            ("api_base", text("https://config")),
            ("api_version", text("2024-01-01")),
            ("aws_bedrock_runtime_endpoint", text("https://bedrock")),
        ]))]
    );
    assert_ne!(
        rows(&result, "healthy_endpoints"),
        rows(&stripped, "healthy_endpoints")
    );
}

#[test]
fn stripping_leaves_the_input_rows_and_non_list_values_as_they_were() {
    let healthy_rows = vec![Json::Object(object(vec![
        ("model_id", text("d1")),
        ("api_base", text("https://config")),
    ]))];
    let result = object(vec![
        ("healthy_endpoints", Json::List(healthy_rows.clone())),
        ("unhealthy_endpoints", Json::Null),
        (
            "other",
            Json::List(vec![Json::Object(object(vec![(
                "api_base",
                text("https://x"),
            )]))]),
        ),
    ]);
    let stripped = strip_admin_only_fields(&result);
    assert_eq!(
        stripped.get("healthy_endpoints"),
        Some(&Json::List(healthy_rows))
    );
    assert_eq!(stripped.get("unhealthy_endpoints"), Some(&Json::Null));
    assert_eq!(stripped.get("other"), result.get("other"));
}

#[test]
fn stripping_keeps_a_non_object_row_untouched() {
    let result = object(vec![(
        "healthy_endpoints",
        Json::List(vec![text("row"), Json::Null, Json::List(Vec::new())]),
    )]);
    let stripped = strip_admin_only_fields(&result);
    assert_eq!(
        stripped.get("healthy_endpoints"),
        Some(&Json::List(vec![
            text("row"),
            Json::Null,
            Json::List(Vec::new()),
        ]))
    );
}

fn cached_health_results() -> JsonObject {
    object(vec![
        (
            "healthy_endpoints",
            Json::List(vec![
                Json::Object(object(vec![
                    ("model_id", text("d1")),
                    ("model", text("gpt")),
                ])),
                Json::Object(object(vec![("model_id", text("d2"))])),
                Json::Object(object(vec![("model_id", text(""))])),
                Json::Object(object(vec![("model", text("no-id"))])),
                Json::Object(object(vec![("model_id", Json::Null)])),
                Json::Object(object(vec![(
                    "model_id",
                    Json::Number(Number::from(7_u64)),
                )])),
                text("not-a-row"),
            ]),
        ),
        (
            "unhealthy_endpoints",
            Json::List(vec![Json::Object(object(vec![
                ("model_id", text("d1")),
                ("error", text("boom")),
            ]))]),
        ),
        ("healthy_count", Json::Number(Number::from(99_u64))),
        ("extra", text("dropped")),
    ])
}

#[test]
fn filtering_keeps_only_rows_with_a_non_empty_allowed_id() {
    let filtered =
        filter_health_check_results_by_model_ids(&cached_health_results(), &names(&["d1"]));
    assert_eq!(
        filtered,
        object(vec![
            (
                "healthy_endpoints",
                Json::List(vec![Json::Object(object(vec![
                    ("model_id", text("d1")),
                    ("model", text("gpt")),
                ]))]),
            ),
            (
                "unhealthy_endpoints",
                Json::List(vec![Json::Object(object(vec![
                    ("model_id", text("d1")),
                    ("error", text("boom")),
                ]))]),
            ),
            ("healthy_count", Json::Number(Number::from(1_u64))),
            ("unhealthy_count", Json::Number(Number::from(1_u64))),
        ])
    );
}

#[test]
fn filtering_leaves_the_shared_results_untouched() {
    let results = cached_health_results();
    let filtered = filter_health_check_results_by_model_ids(&results, &names(&["d1"]));
    assert_eq!(
        results.get("healthy_count"),
        Some(&Json::Number(Number::from(99_u64)))
    );
    assert!(results.contains_key("extra"));
    assert_ne!(
        results.get("healthy_endpoints"),
        filtered.get("healthy_endpoints")
    );
}

#[test]
fn filtering_a_missing_or_null_list_gives_empty_counts() {
    let empty = filter_health_check_results_by_model_ids(&JsonObject::new(), &names(&["d1"]));
    assert_eq!(
        empty,
        object(vec![
            ("healthy_endpoints", Json::List(Vec::new())),
            ("unhealthy_endpoints", Json::List(Vec::new())),
            ("healthy_count", Json::Number(Number::from(0_u64))),
            ("unhealthy_count", Json::Number(Number::from(0_u64))),
        ])
    );
    let nulled = object(vec![
        ("healthy_endpoints", Json::Null),
        ("unhealthy_endpoints", text("not a list")),
    ]);
    let filtered = filter_health_check_results_by_model_ids(&nulled, &names(&["d1"]));
    assert_eq!(
        filtered.get("healthy_count"),
        Some(&Json::Number(Number::from(0_u64)))
    );
    assert_eq!(
        filtered.get("unhealthy_endpoints"),
        Some(&Json::List(Vec::new()))
    );
}

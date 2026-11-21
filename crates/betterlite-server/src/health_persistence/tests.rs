#![cfg(test)]

mod support;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use serde_json::Number;

use super::{
    AggregatedHealthResult, HealthCycle, ModelParam, ModelParamIndex, SaveRequest,
    SingleHealthCheck, aggregate_results, model_param_index, new_status, save_background,
    save_changed, save_single, should_persist,
};
use support::{
    BROKEN_LATEST, FakeStore, FixedClock, ScriptedClock, ago, at, capture_logs, deployment,
    deployment_json, endpoint, latest_map, latest_row, result_of,
};

#[test]
fn index_groups_every_model_under_its_model_param() {
    let deployments = vec![
        deployment("model-a", "shared", Some("dep-a")),
        deployment("model-b", "shared", Some("dep-b")),
        deployment("model-c", "own", None),
    ];
    let index = model_param_index(&deployments);
    assert_eq!(index.len(), 2);
    assert_eq!(
        index.get("shared").cloned(),
        Some(vec![
            ModelParam {
                model_name: String::from("model-a"),
                model_id: Some(String::from("dep-a")),
            },
            ModelParam {
                model_name: String::from("model-b"),
                model_id: Some(String::from("dep-b")),
            },
        ])
    );
    assert_eq!(
        index.get("own").cloned(),
        Some(vec![ModelParam {
            model_name: String::from("model-c"),
            model_id: None,
        }])
    );
}

#[test]
fn index_skips_a_deployment_without_a_usable_model_param() {
    let mut params = JsonObject::new();
    params.insert(
        String::from("api_base"),
        Json::Text(String::from("http://x")),
    );
    let without_param = DeploymentSpec {
        model_name: String::from("none"),
        litellm_params: params,
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    };
    let deployments = vec![
        without_param,
        deployment("", "empty-name", None),
        deployment("empty-param", "", None),
        deployment_json("null-param", Json::Null, None),
        deployment_json("number-param", Json::Number(Number::from(1_u8)), None),
    ];
    assert!(model_param_index(&deployments).is_empty());
}

#[test]
fn index_treats_a_non_text_or_empty_model_id_as_missing() {
    let deployments = vec![
        deployment_json("number", "p1", Some(Json::Number(Number::from(7_u8)))),
        deployment_json("empty", "p2", Some(Json::Text(String::new()))),
        deployment_json("null", "p3", Some(Json::Null)),
        deployment("absent", "p4", None),
        deployment("text", "p5", Some("dep-1")),
    ];
    let index = model_param_index(&deployments);
    let listed: Vec<(&str, Option<&str>)> = index
        .iter()
        .map(|(param, models)| (param.as_str(), models.first().unwrap().model_id.as_deref()))
        .collect();
    assert_eq!(
        listed,
        [
            ("p1", None),
            ("p2", None),
            ("p3", None),
            ("p4", None),
            ("p5", Some("dep-1")),
        ]
    );
}

#[test]
fn aggregate_counts_a_shared_param_once_per_model() {
    let mut index = ModelParamIndex::new();
    index.insert(
        String::from("shared"),
        vec![
            ModelParam {
                model_name: String::from("model-a"),
                model_id: Some(String::from("dep-a")),
            },
            ModelParam {
                model_name: String::from("model-b"),
                model_id: None,
            },
        ],
    );
    let healthy = vec![endpoint("shared", None), endpoint("shared", None)];
    let results = aggregate_results(&index, &healthy, &[]);
    assert_eq!(
        results,
        vec![
            result_of("model-a", Some("dep-a"), (2, 0)),
            result_of("model-b", None, (2, 0)),
        ]
    );
}

#[test]
fn aggregate_ignores_an_endpoint_for_an_unknown_param() {
    let mut index = ModelParamIndex::new();
    index.insert(
        String::from("known"),
        vec![ModelParam {
            model_name: String::from("model-a"),
            model_id: None,
        }],
    );
    let healthy = vec![endpoint("unknown", None)];
    let unhealthy = vec![endpoint("unknown", Some("boom"))];
    assert!(aggregate_results(&index, &healthy, &unhealthy).is_empty());
}

#[test]
fn aggregate_orders_healthy_first_then_unhealthy_only_keys() {
    let mut index = ModelParamIndex::new();
    index.insert(
        String::from("p-healthy"),
        vec![ModelParam {
            model_name: String::from("healthy-model"),
            model_id: None,
        }],
    );
    index.insert(
        String::from("p-unhealthy"),
        vec![ModelParam {
            model_name: String::from("unhealthy-model"),
            model_id: None,
        }],
    );
    let healthy = vec![endpoint("p-healthy", None)];
    let unhealthy = vec![endpoint("p-unhealthy", Some("boom"))];
    let results = aggregate_results(&index, &healthy, &unhealthy);
    let names: Vec<&str> = results
        .iter()
        .map(|result| result.model_name.as_str())
        .collect();
    assert_eq!(names, ["healthy-model", "unhealthy-model"]);
}

#[test]
fn aggregate_counts_every_unhealthy_endpoint_and_keeps_the_first_error() {
    let mut index = ModelParamIndex::new();
    index.insert(
        String::from("param"),
        vec![ModelParam {
            model_name: String::from("model-a"),
            model_id: None,
        }],
    );
    let unhealthy = vec![
        endpoint("param", Some("")),
        endpoint("param", Some("second")),
        endpoint("param", Some("third")),
    ];
    let results = aggregate_results(&index, &[], &unhealthy);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].unhealthy_count, 3_i32);
    assert_eq!(results[0].error_message.as_deref(), Some("second"));
}

#[test]
fn aggregate_cuts_the_error_to_500_chars() {
    let mut index = ModelParamIndex::new();
    index.insert(
        String::from("param"),
        vec![ModelParam {
            model_name: String::from("model-a"),
            model_id: None,
        }],
    );
    let long = "x".repeat(600);
    let unhealthy = vec![endpoint("param", Some(&long))];
    let results = aggregate_results(&index, &[], &unhealthy);
    assert_eq!(results.len(), 1);
    let error = results[0].error_message.as_deref().unwrap();
    assert_eq!(error.len(), 500);
}

#[test]
fn aggregate_writes_a_non_text_error_as_json() {
    let mut index = ModelParamIndex::new();
    index.insert(
        String::from("param"),
        vec![ModelParam {
            model_name: String::from("model-a"),
            model_id: None,
        }],
    );
    let mut failure = JsonObject::new();
    failure.insert("model".to_owned(), Json::Text(String::from("param")));
    let mut detail = JsonObject::new();
    detail.insert("detail".to_owned(), Json::Text(String::from("boom")));
    failure.insert("error".to_owned(), Json::Object(detail));
    let results = aggregate_results(&index, &[], &[failure]);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].error_message.as_deref(),
        Some(r#"{"detail":"boom"}"#)
    );
}

#[test]
fn decision_status_is_healthy_with_any_healthy_endpoint() {
    assert_eq!(new_status(&result_of("model-a", None, (1, 3))), "healthy");
    assert_eq!(new_status(&result_of("model-a", None, (0, 3))), "unhealthy");
}

#[test]
fn decision_returns_true_without_a_previous_row() {
    let latest = latest_map(Vec::new());
    assert!(should_persist(
        &result_of("model-a", Some("dep-a"), (1, 0)),
        &latest,
        at(1000)
    ));
}

#[test]
fn decision_returns_true_when_the_status_changed() {
    let latest = latest_map(vec![latest_row(
        "model-a",
        Some("dep-a"),
        "unhealthy",
        at(1000),
    )]);
    assert!(should_persist(
        &result_of("model-a", Some("dep-a"), (1, 0)),
        &latest,
        at(1001)
    ));
}

#[test]
fn decision_returns_true_at_exactly_one_hour() {
    let latest = latest_map(vec![latest_row(
        "model-a",
        Some("dep-a"),
        "healthy",
        at(1000),
    )]);
    assert!(should_persist(
        &result_of("model-a", Some("dep-a"), (1, 0)),
        &latest,
        at(4600)
    ));
}

#[test]
fn decision_returns_false_just_under_one_hour() {
    let now = at(1000);
    let latest = latest_map(vec![latest_row(
        "model-a",
        Some("dep-a"),
        "healthy",
        ago(now, 3599),
    )]);
    assert!(!should_persist(
        &result_of("model-a", Some("dep-a"), (1, 0)),
        &latest,
        now
    ));
}

#[test]
fn decision_prefers_the_model_id_key_when_it_has_text() {
    // the id key's row is over an hour old and the name key's is one second old, so the two keys disagree
    let now = at(7200);
    let latest = latest_map(vec![
        latest_row("model-a", Some("dep-a"), "healthy", at(0)),
        latest_row("model-a", None, "healthy", ago(now, 1)),
    ]);
    assert!(should_persist(
        &result_of("model-a", Some("dep-a"), (1, 0)),
        &latest,
        now
    ));
}

#[test]
fn decision_falls_back_to_the_name_for_an_empty_or_missing_id() {
    let now = at(1000);
    let latest = latest_map(vec![latest_row("model-a", None, "healthy", ago(now, 1))]);
    assert!(!should_persist(
        &result_of("model-a", Some(""), (1, 0)),
        &latest,
        now
    ));
    assert!(!should_persist(
        &result_of("model-a", None, (1, 0)),
        &latest,
        now
    ));
    // a stored row with an empty model_id is keyed by its name, as the result is
    let empty_id = latest_map(vec![latest_row(
        "model-a",
        Some(""),
        "healthy",
        ago(now, 1),
    )]);
    assert!(!should_persist(
        &result_of("model-a", Some(""), (1, 0)),
        &empty_id,
        now
    ));
}

#[tokio::test]
async fn changed_writes_only_the_results_that_should_persist() {
    let now = at(1000);
    let latest = latest_map(vec![latest_row(
        "model-b",
        Some("dep-b"),
        "healthy",
        ago(now, 1),
    )]);
    let store = FakeStore::new(Vec::new());
    let results = vec![
        result_of("model-a", Some("dep-a"), (1, 0)),
        result_of("model-b", Some("dep-b"), (1, 0)),
    ];
    let request = SaveRequest {
        start_time: at(999),
        checked_by: None,
    };
    let saved = save_changed(&store, &results, &latest, &request, &FixedClock(now)).await;
    assert!(saved);
    let written = store.saved();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].model_name, "model-a");
}

#[tokio::test]
async fn changed_runs_every_write_together() {
    let store = FakeStore::new(Vec::new());
    let results = vec![
        result_of("model-a", None, (1, 0)),
        result_of("model-b", None, (1, 0)),
    ];
    let request = SaveRequest {
        start_time: at(999),
        checked_by: Some("health"),
    };
    let latest = latest_map(Vec::new());
    let saved = save_changed(&store, &results, &latest, &request, &FixedClock(at(1000))).await;
    assert!(saved);
    assert_eq!(store.saved().len(), 2);
    assert_eq!(store.max_in_flight(), 2);
    assert_eq!(store.in_flight(), 0);
}

#[tokio::test]
async fn changed_reads_the_clock_before_and_after_each_pick() {
    let clock = ScriptedClock::new(vec![at(1000), at(1000), at(1001), at(1002), at(1005)]);
    let latest = latest_map(vec![latest_row(
        "model-b",
        Some("dep-b"),
        "healthy",
        at(1000),
    )]);
    let store = FakeStore::new(Vec::new());
    let results = vec![
        result_of("model-a", Some("dep-a"), (1, 0)),
        result_of("model-b", Some("dep-b"), (1, 0)),
        result_of("model-c", None, (0, 1)),
    ];
    let request = SaveRequest {
        start_time: at(999),
        checked_by: Some("health"),
    };
    let saved = save_changed(&store, &results, &latest, &request, &clock).await;
    assert!(saved);
    assert_eq!(clock.calls(), 5);
    let written = store.saved();
    assert_eq!(written.len(), 2);
    assert_eq!(written[0].model_name, "model-a");
    assert_eq!(written[0].response_time_ms, Some(1000.0_f64));
    assert_eq!(written[1].model_name, "model-c");
    assert_eq!(written[1].response_time_ms, Some(6000.0_f64));
}

#[tokio::test]
async fn changed_returns_false_when_one_write_fails() {
    let store = FakeStore::new(Vec::new()).failing_for(&["bad-model"]);
    let results = vec![
        result_of("bad-model", None, (1, 0)),
        result_of("good-model", None, (1, 0)),
    ];
    let request = SaveRequest {
        start_time: at(999),
        checked_by: None,
    };
    let latest = latest_map(Vec::new());
    let saved = save_changed(&store, &results, &latest, &request, &FixedClock(at(1000))).await;
    assert!(!saved);
    let written = store.saved();
    assert_eq!(written.len(), 2);
    assert_eq!(written[0].model_name, "bad-model");
    assert_eq!(written[1].model_name, "good-model");
}

#[tokio::test]
async fn changed_returns_true_when_there_is_nothing_to_write() {
    let clock = ScriptedClock::new(Vec::new());
    let store = FakeStore::new(Vec::new());
    let latest = latest_map(Vec::new());
    let request = SaveRequest {
        start_time: at(999),
        checked_by: None,
    };
    let saved = save_changed(&store, &[], &latest, &request, &clock).await;
    assert!(saved);
    assert!(store.saved().is_empty());
    assert_eq!(clock.calls(), 0);
}

#[tokio::test]
async fn changed_copies_the_request_into_every_write() {
    let store = FakeStore::new(Vec::new());
    let results = vec![AggregatedHealthResult {
        error_message: Some(String::from("boom")),
        ..result_of("model-a", Some("dep-a"), (2, 1))
    }];
    let latest = latest_map(Vec::new());
    let request = SaveRequest {
        start_time: at(1000),
        checked_by: Some("user-7"),
    };
    let saved = save_changed(&store, &results, &latest, &request, &FixedClock(at(1001))).await;
    assert!(saved);
    let written = store.saved();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].model_name, "model-a");
    assert_eq!(written[0].model_id.as_deref(), Some("dep-a"));
    assert_eq!(written[0].status, "healthy");
    assert_eq!(written[0].healthy_count, 2_i32);
    assert_eq!(written[0].unhealthy_count, 1_i32);
    assert_eq!(written[0].error_message.as_deref(), Some("boom"));
    assert_eq!(written[0].checked_by.as_deref(), Some("user-7"));
    assert_eq!(written[0].response_time_ms, Some(1000.0_f64));
}

#[tokio::test]
async fn background_returns_false_without_a_store() {
    let deployments = vec![deployment("gpt-4o", "shared", Some("dep-1"))];
    let healthy = vec![endpoint("shared", None)];
    let cycle = HealthCycle {
        deployments: &deployments,
        healthy: &healthy,
        unhealthy: &[],
    };
    let request = SaveRequest {
        start_time: at(1000),
        checked_by: None,
    };
    let saved = save_background::<FakeStore>(None, cycle, &request, &FixedClock(at(1001))).await;
    assert!(!saved);
}

#[tokio::test]
async fn background_logs_and_gives_up_when_the_latest_query_fails() {
    let (logs, _guard) = capture_logs();
    let store = FakeStore::with_broken_latest();
    let deployments = vec![deployment("gpt-4o", "shared", Some("dep-1"))];
    let healthy = vec![endpoint("shared", None)];
    let cycle = HealthCycle {
        deployments: &deployments,
        healthy: &healthy,
        unhealthy: &[],
    };
    let request = SaveRequest {
        start_time: at(1000),
        checked_by: None,
    };
    let saved = save_background(Some(&store), cycle, &request, &FixedClock(at(1001))).await;
    assert!(!saved);
    assert_eq!(store.latest_calls(), 1);
    assert!(store.saved().is_empty());
    assert!(logs.text().contains(&format!(
        "Failed to save background health checks to database: {BROKEN_LATEST}"
    )));
}

#[tokio::test]
async fn background_writes_a_first_seen_model() {
    let store = FakeStore::new(Vec::new());
    let deployments = vec![deployment("gpt-4o", "shared", Some("dep-1"))];
    let healthy = vec![endpoint("shared", None)];
    let cycle = HealthCycle {
        deployments: &deployments,
        healthy: &healthy,
        unhealthy: &[],
    };
    let request = SaveRequest {
        start_time: at(1000),
        checked_by: Some("health-cycle"),
    };
    let saved = save_background(Some(&store), cycle, &request, &FixedClock(at(1001))).await;
    assert!(saved);
    let written = store.saved();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].model_name, "gpt-4o");
    assert_eq!(written[0].model_id.as_deref(), Some("dep-1"));
    assert_eq!(written[0].status, "healthy");
    assert_eq!(written[0].healthy_count, 1_i32);
    assert_eq!(written[0].unhealthy_count, 0_i32);
    assert!(written[0].error_message.is_none());
    assert_eq!(written[0].checked_by.as_deref(), Some("health-cycle"));
    assert_eq!(written[0].response_time_ms, Some(1000.0_f64));
}

#[tokio::test]
async fn background_keeps_the_first_row_per_key() {
    let store = FakeStore::new(vec![
        latest_row("gpt-4o", Some("dep-1"), "healthy", at(1000)),
        latest_row("gpt-4o", Some("dep-1"), "unhealthy", at(0)),
    ]);
    let deployments = vec![deployment("gpt-4o", "shared", Some("dep-1"))];
    let healthy = vec![endpoint("shared", None)];
    let cycle = HealthCycle {
        deployments: &deployments,
        healthy: &healthy,
        unhealthy: &[],
    };
    let request = SaveRequest {
        start_time: at(1000),
        checked_by: None,
    };
    let saved = save_background(Some(&store), cycle, &request, &FixedClock(at(1001))).await;
    assert!(saved);
    assert!(store.saved().is_empty());
}

#[tokio::test]
async fn background_skips_a_recent_unchanged_status() {
    let store = FakeStore::new(vec![latest_row(
        "gpt-4o",
        Some("dep-1"),
        "unhealthy",
        at(1000),
    )]);
    let deployments = vec![deployment("gpt-4o", "shared", Some("dep-1"))];
    let unhealthy = vec![endpoint("shared", Some("boom"))];
    let cycle = HealthCycle {
        deployments: &deployments,
        healthy: &[],
        unhealthy: &unhealthy,
    };
    let request = SaveRequest {
        start_time: at(1000),
        checked_by: None,
    };
    let saved = save_background(Some(&store), cycle, &request, &FixedClock(at(1001))).await;
    assert!(saved);
    assert!(store.saved().is_empty());
}

#[tokio::test]
async fn single_writes_one_row_from_the_healthy_list() {
    let store = FakeStore::new(Vec::new());
    let healthy = vec![endpoint("m", None)];
    let check = SingleHealthCheck {
        model_name: "gpt-4o",
        model_id: Some("dep-1"),
        healthy: &healthy,
        unhealthy: &[],
        start_time: at(1000),
        user_id: Some("user-1"),
    };
    save_single(&store, &check, &FixedClock(at(1001))).await;
    let written = store.saved();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].model_name, "gpt-4o");
    assert_eq!(written[0].model_id.as_deref(), Some("dep-1"));
    assert_eq!(written[0].status, "healthy");
    assert_eq!(written[0].healthy_count, 1_i32);
    assert_eq!(written[0].unhealthy_count, 0_i32);
    assert!(written[0].error_message.is_none());
    assert_eq!(written[0].checked_by.as_deref(), Some("user-1"));
    assert_eq!(written[0].response_time_ms, Some(1000.0_f64));
}

#[tokio::test]
async fn single_writes_unhealthy_counts_and_the_first_error() {
    let store = FakeStore::new(Vec::new());
    let unhealthy = vec![endpoint("m", Some("first")), endpoint("m", Some("second"))];
    let check = SingleHealthCheck {
        model_name: "gpt-4o",
        model_id: None,
        healthy: &[],
        unhealthy: &unhealthy,
        start_time: at(1000),
        user_id: None,
    };
    save_single(&store, &check, &FixedClock(at(1001))).await;
    let written = store.saved();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].status, "unhealthy");
    assert_eq!(written[0].healthy_count, 0_i32);
    assert_eq!(written[0].unhealthy_count, 2_i32);
    assert_eq!(written[0].error_message.as_deref(), Some("first"));
    assert!(written[0].checked_by.is_none());
}

#[tokio::test]
async fn single_writes_unhealthy_without_any_endpoint() {
    let store = FakeStore::new(Vec::new());
    let check = SingleHealthCheck {
        model_name: "gpt-4o",
        model_id: None,
        healthy: &[],
        unhealthy: &[],
        start_time: at(1000),
        user_id: None,
    };
    save_single(&store, &check, &FixedClock(at(1001))).await;
    let written = store.saved();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].status, "unhealthy");
    assert_eq!(written[0].healthy_count, 0_i32);
    assert_eq!(written[0].unhealthy_count, 0_i32);
    assert!(written[0].error_message.is_none());
}

#[tokio::test]
async fn single_cuts_the_error_to_500_chars() {
    let store = FakeStore::new(Vec::new());
    let long = "x".repeat(600);
    let unhealthy = vec![endpoint("m", Some(&long))];
    let check = SingleHealthCheck {
        model_name: "gpt-4o",
        model_id: None,
        healthy: &[],
        unhealthy: &unhealthy,
        start_time: at(1000),
        user_id: None,
    };
    save_single(&store, &check, &FixedClock(at(1001))).await;
    let written = store.saved();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].error_message.as_deref().unwrap().len(), 500);
}

#[tokio::test]
async fn single_ignores_a_failed_write() {
    let store = FakeStore::new(Vec::new()).failing_for(&["gpt-4o"]);
    let healthy = vec![endpoint("m", None)];
    let check = SingleHealthCheck {
        model_name: "gpt-4o",
        model_id: None,
        healthy: &healthy,
        unhealthy: &[],
        start_time: at(1000),
        user_id: None,
    };
    save_single(&store, &check, &FixedClock(at(1001))).await;
    assert_eq!(store.saved().len(), 1);
}

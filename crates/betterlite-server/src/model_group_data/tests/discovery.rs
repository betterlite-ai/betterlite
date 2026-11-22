use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;

use super::super::discovery::{
    get_all_access_groups_from_db, missing_after_read_through, require_group_exists,
    validate_models_exist,
};
use super::fixtures::json_object;
use super::fixtures::router::{FailingRouter, ScriptedRouter};
use super::fixtures::scripted::ScriptedDatabase;
use super::{
    deployment, group_database, http_failure, linked_group, names, number, runtime_message,
    tagged_deployment, text,
};

#[tokio::test]
async fn group_count_duplicates() {
    let database = group_database(vec![
        tagged_deployment("d1", "beta", &["group-a", "group-a"]),
        tagged_deployment("d2", "alpha", &["group-a"]),
        tagged_deployment("d3", "beta", &["group-a", "group-b"]),
        deployment("d4", "delta", None),
        deployment("d5", "epsilon", Some(Json::Object(JsonObject::new()))),
    ]);
    let groups = get_all_access_groups_from_db(&database)
        .await
        .expect("the access groups");
    assert_eq!(groups.len(), 2);
    let group_a = groups.get("group-a").expect("the seeded group");
    assert_eq!(group_a.model_names, names(&["alpha", "beta"]));
    assert_eq!(group_a.deployment_count, 4);
    assert_eq!(group_a.spend, None);
    let group_b = groups.get("group-b").expect("the seeded group");
    assert_eq!(group_b.model_names, names(&["beta"]));
    assert_eq!(group_b.deployment_count, 1);
}

#[tokio::test]
async fn discovery_bad_row_failure() {
    let text_info = group_database(vec![deployment(
        "d1",
        "alpha",
        Some(Json::Text("not json".to_owned())),
    )]);
    let error = get_all_access_groups_from_db(&text_info)
        .await
        .expect_err("the unreadable row");
    assert_eq!(
        runtime_message(error),
        "model_info for deployment d1 is not a JSON object"
    );
    let string_groups = group_database(vec![deployment(
        "d2",
        "beta",
        Some(Json::Object(json_object(&[(
            "access_groups",
            text("group-a"),
        )]))),
    )]);
    let error = get_all_access_groups_from_db(&string_groups)
        .await
        .expect_err("the malformed groups");
    assert_eq!(
        runtime_message(error),
        "access_groups for deployment d2 is not a list"
    );
    let number_group = group_database(vec![deployment(
        "d3",
        "gamma",
        Some(Json::Object(json_object(&[(
            "access_groups",
            Json::List(vec![number(1.0)]),
        )]))),
    )]);
    let error = get_all_access_groups_from_db(&number_group)
        .await
        .expect_err("the malformed entry");
    assert_eq!(
        runtime_message(error),
        "access_groups for deployment d3 holds a non-string entry"
    );
}

#[tokio::test]
async fn group_existence_from_tags() {
    let tagged = group_database(vec![tagged_deployment("d1", "alpha", &["group-a"])]);
    require_group_exists("group-a", &tagged)
        .await
        .expect("the tagged group");
    let error = require_group_exists("other", &tagged)
        .await
        .expect_err("the missing group");
    let failure = http_failure(error);
    assert_eq!(failure.status, StatusCode::NOT_FOUND);
    assert_eq!(failure.message, "Access group 'other' not found");
    let budget_only = ScriptedDatabase::new(
        Vec::new(),
        Vec::new(),
        vec![linked_group("group-a", Some(Decimal::ZERO), None, None)],
    );
    let error = require_group_exists("group-a", &budget_only)
        .await
        .expect_err("the budget row alone");
    assert_eq!(http_failure(error).status, StatusCode::NOT_FOUND);
}

#[test]
fn exact_model_names() {
    let router = ScriptedRouter::new(names(&["gpt-4", "gpt-4o"]), Vec::new());
    assert_eq!(
        validate_models_exist(&names(&["gpt-4"]), Some(&router)).expect("the names"),
        (true, Vec::new())
    );
    assert_eq!(
        validate_models_exist(&names(&["gpt-4o-mini"]), Some(&router)).expect("the names"),
        (false, names(&["gpt-4o-mini"]))
    );
    assert_eq!(
        validate_models_exist(&names(&["x", "gpt-4", "y", "x"]), Some(&router)).expect("the names"),
        (false, names(&["x", "y", "x"]))
    );
    assert_eq!(
        validate_models_exist(&names(&["x", "x"]), None::<&ScriptedRouter>).expect("the names"),
        (false, names(&["x", "x"]))
    );
}

#[test]
fn lookup_failure_propagates() {
    let error = validate_models_exist(&names(&["alpha"]), Some(&FailingRouter))
        .expect_err("the stale catalog");
    assert_eq!(
        error.to_string(),
        "the catalog index points at row 3, which is not in the model list"
    );
}

#[tokio::test]
async fn readthrough_order_and_rebound_router() {
    let router = ScriptedRouter::new(names(&["alpha"]), names(&["beta"]));
    let remaining =
        missing_after_read_through(&names(&["alpha", "beta", "gamma"]), &router, Some(&router))
            .await
            .expect("the missing names");
    assert_eq!(remaining, names(&["gamma"]));
    assert_eq!(router.attempted(), names(&["beta", "gamma"]));
}

#[tokio::test]
async fn readthrough_empty_missing_no_work() {
    let router = ScriptedRouter::new(names(&["alpha"]), Vec::new());
    let remaining = missing_after_read_through(&names(&["alpha"]), &router, Some(&router))
        .await
        .expect("the missing names");
    assert_eq!(remaining, Vec::<String>::new());
    assert!(router.attempted().is_empty());
}

#[tokio::test]
async fn readthrough_installs_a_router() {
    let router = ScriptedRouter::absent(Vec::new(), names(&["beta"]));
    let remaining = missing_after_read_through(&names(&["beta"]), &router, Some(&router))
        .await
        .expect("the missing names");
    assert_eq!(remaining, Vec::<String>::new());
    assert_eq!(router.attempted(), names(&["beta"]));
}

#[tokio::test]
async fn readthrough_failure_no_write() {
    let database = group_database(vec![deployment("d1", "alpha", None)]);
    let router = ScriptedRouter::new(Vec::new(), Vec::new());
    let remaining = missing_after_read_through(&names(&["alpha"]), &router, Some(&router))
        .await
        .expect("the missing names");
    assert_eq!(remaining, names(&["alpha"]));
    assert_eq!(router.attempted(), names(&["alpha"]));
    assert!(database.updates().is_empty());
}

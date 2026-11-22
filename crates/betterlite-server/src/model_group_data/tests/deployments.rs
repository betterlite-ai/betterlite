use axum::http::StatusCode;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::super::deployments::{
    DeploymentSelection, GroupChange, add_access_group_to_deployment, find_deployment_info_or_400,
    readable_model_info_or_raise, remove_group_from_deployment, tag_deployments,
    update_deployment_group,
};
use super::fixtures::json_object;
use super::{
    deployment, group_database, http_failure, list, names, number, tagged_deployment, text,
    value_error_message,
};

#[test]
fn add_idempotent_remove_first() {
    let mut absent = JsonObject::new();
    assert!(add_access_group_to_deployment(&mut absent, "group-a").expect("the add"));
    assert_eq!(absent.get("access_groups"), Some(&list(&["group-a"])));
    assert!(!add_access_group_to_deployment(&mut absent, "group-a").expect("the second add"));
    assert_eq!(absent.get("access_groups"), Some(&list(&["group-a"])));
    let mut twice = json_object(&[("access_groups", list(&["group-a", "group-a"]))]);
    assert!(remove_group_from_deployment(&mut twice, "group-a").expect("the remove"));
    assert_eq!(twice.get("access_groups"), Some(&list(&["group-a"])));
    let mut only = json_object(&[("access_groups", list(&["group-a"]))]);
    assert!(remove_group_from_deployment(&mut only, "group-a").expect("the remove"));
    assert_eq!(only.get("access_groups"), Some(&Json::List(Vec::new())));
    let mut without = json_object(&[("id", text("d1"))]);
    assert!(!remove_group_from_deployment(&mut without, "group-a").expect("the remove"));
    assert_eq!(without.get("access_groups"), None);
    let mut empty = json_object(&[("access_groups", Json::List(Vec::new()))]);
    assert!(!remove_group_from_deployment(&mut empty, "group-a").expect("the remove"));
    assert_eq!(empty.get("access_groups"), Some(&Json::List(Vec::new())));
}

#[test]
fn malformed_groups() {
    let mut info = json_object(&[(
        "access_groups",
        Json::List(vec![Json::Null, text("group-a"), number(1.0)]),
    )]);
    assert!(!add_access_group_to_deployment(&mut info, "group-a").expect("the add"));
    assert!(add_access_group_to_deployment(&mut info, "group-b").expect("the add"));
    assert_eq!(
        info.get("access_groups"),
        Some(&Json::List(vec![
            Json::Null,
            text("group-a"),
            number(1.0),
            text("group-b"),
        ]))
    );
    assert!(remove_group_from_deployment(&mut info, "group-a").expect("the remove"));
    assert_eq!(
        info.get("access_groups"),
        Some(&Json::List(vec![Json::Null, number(1.0), text("group-b"),]))
    );
}

#[test]
fn readable_json_or_absent() {
    assert_eq!(
        readable_model_info_or_raise("d1", None).expect("the empty start"),
        JsonObject::new()
    );
    let null = Json::Null;
    assert_eq!(
        readable_model_info_or_raise("d1", Some(&null)).expect("the empty start"),
        JsonObject::new()
    );
    let object = json_object(&[("id", text("d1"))]);
    assert_eq!(
        readable_model_info_or_raise("d1", Some(&Json::Object(object.clone())))
            .expect("the stored object"),
        object
    );
    let stored_text = Json::Text(r#"{"id": "d1"}"#.to_owned());
    assert_eq!(
        readable_model_info_or_raise("d1", Some(&stored_text)).expect("the stored text"),
        object
    );
    let broken = readable_model_info_or_raise("d1", Some(&Json::Text("not json".to_owned())))
        .expect_err("the unreadable value");
    assert_eq!(
        value_error_message(broken),
        "model_info for deployment d1 is not a readable JSON object; refusing to rewrite it"
    );
    let listed = readable_model_info_or_raise("d2", Some(&Json::List(Vec::new())))
        .expect_err("the unreadable value");
    assert_eq!(
        value_error_message(listed),
        "model_info for deployment d2 is not a readable JSON object; refusing to rewrite it"
    );
}

#[tokio::test]
async fn unreadable_refuses_write() {
    let stored = Json::Text("not json".to_owned());
    let database = group_database(vec![deployment("d1", "alpha", Some(stored.clone()))]);
    let error =
        update_deployment_group("d1", Some(&stored), "group-a", GroupChange::Add, &database)
            .await
            .expect_err("the refused write");
    assert_eq!(
        value_error_message(error),
        "model_info for deployment d1 is not a readable JSON object; refusing to rewrite it"
    );
    assert!(database.updates().is_empty());
    assert_eq!(database.deployment_info("d1"), Some(stored));
}

#[tokio::test]
async fn preserve_other_model_info() {
    let stored = Json::Object(json_object(&[
        ("id", text("d1")),
        ("ciphertext", text("c2VjcmV0")),
        ("db_model", Json::Bool(true)),
    ]));
    let database = group_database(vec![deployment("d1", "alpha", Some(stored.clone()))]);
    let pair = update_deployment_group("d1", Some(&stored), "group-a", GroupChange::Add, &database)
        .await
        .expect("the tag")
        .expect("a written pair");
    let expected = json_object(&[
        ("id", text("d1")),
        ("ciphertext", text("c2VjcmV0")),
        ("db_model", Json::Bool(true)),
        ("access_groups", list(&["group-a"])),
    ]);
    assert_eq!(pair.0, "d1");
    assert_eq!(pair.1, expected);
    assert_eq!(
        database.deployment_info("d1"),
        Some(Json::Object(expected.clone()))
    );
    let updates = database.updates();
    assert_eq!(updates.len(), 1);
    let written: Json = serde_json::from_str(&updates[0].1).expect("the written JSON");
    assert_eq!(written, Json::Object(expected));
}

#[tokio::test]
async fn tag_skips_unchanged_write() {
    let stored = tagged_deployment("d1", "alpha", &["group-a"]);
    let database = group_database(vec![stored.clone()]);
    let pair = update_deployment_group(
        "d1",
        stored.model_info.as_ref(),
        "group-a",
        GroupChange::Add,
        &database,
    )
    .await
    .expect("the tag");
    assert_eq!(pair, None);
    assert!(database.updates().is_empty());
}

#[tokio::test]
async fn strip_skips_unchanged_write() {
    let stored = tagged_deployment("d1", "alpha", &["group-b"]);
    let database = group_database(vec![stored.clone()]);
    let pair = update_deployment_group(
        "d1",
        stored.model_info.as_ref(),
        "group-a",
        GroupChange::Remove,
        &database,
    )
    .await
    .expect("the strip");
    assert_eq!(pair, None);
    assert!(database.updates().is_empty());
}

#[tokio::test]
async fn strip_returns_updated_pair() {
    let stored = tagged_deployment("d1", "alpha", &["group-a", "group-b"]);
    let database = group_database(vec![stored.clone()]);
    let pair = update_deployment_group(
        "d1",
        stored.model_info.as_ref(),
        "group-a",
        GroupChange::Remove,
        &database,
    )
    .await
    .expect("the strip")
    .expect("a written pair");
    assert_eq!(
        pair.1,
        json_object(&[("access_groups", list(&["group-b"]))])
    );
    assert_eq!(database.deployment_info("d1"), Some(Json::Object(pair.1)));
    assert_eq!(database.updates().len(), 1);
}

#[tokio::test]
async fn names_validate_all_before_writes() {
    let database = group_database(vec![
        deployment("d1", "alpha", None),
        deployment("d2", "beta", None),
    ]);
    let model_names = names(&["alpha", "missing"]);
    let error = tag_deployments(
        DeploymentSelection::ByNames(&model_names),
        "group-a",
        &database,
    )
    .await
    .expect_err("the missing name");
    let failure = http_failure(error);
    assert_eq!(failure.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        failure.message,
        "Can't find model 'missing' in Database. Access group management is only supported for database models."
    );
    assert!(database.updates().is_empty());
}

#[tokio::test]
async fn names_write_in_query_order() {
    let database = group_database(vec![
        deployment("d2", "beta", None),
        deployment("d1", "alpha", None),
    ]);
    let model_names = names(&["alpha", "beta", "alpha"]);
    let tagged = tag_deployments(
        DeploymentSelection::ByNames(&model_names),
        "group-a",
        &database,
    )
    .await
    .expect("the tags");
    let written: Vec<String> = tagged.iter().map(|pair| pair.0.clone()).collect();
    assert_eq!(written, names(&["d2", "d1"]));
    let updated: Vec<String> = database
        .updates()
        .into_iter()
        .map(|update| update.0)
        .collect();
    assert_eq!(updated, names(&["d2", "d1"]));
    assert_eq!(
        database.deployment_info("d1"),
        Some(Json::Object(json_object(&[(
            "access_groups",
            list(&["group-a"]),
        )])))
    );
}

#[tokio::test]
async fn ids_partial_write_on_missing() {
    let database = group_database(vec![
        deployment("d1", "alpha", None),
        deployment("d3", "gamma", None),
    ]);
    let model_ids = names(&["d1", "d2", "d3"]);
    let error = tag_deployments(DeploymentSelection::ByIds(&model_ids), "group-a", &database)
        .await
        .expect_err("the missing id");
    let failure = http_failure(error);
    assert_eq!(failure.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        failure.message,
        "Deployment with model_id 'd2' not found in Database."
    );
    let updated: Vec<String> = database
        .updates()
        .into_iter()
        .map(|update| update.0)
        .collect();
    assert_eq!(updated, names(&["d1"]));
    assert!(database.deployment_info("d3").is_none());
}

#[tokio::test]
async fn duplicate_id_rechecks() {
    let database = group_database(vec![deployment("d1", "alpha", None)]);
    let model_ids = names(&["d1", "d1"]);
    let tagged = tag_deployments(DeploymentSelection::ByIds(&model_ids), "group-a", &database)
        .await
        .expect("the tags");
    assert_eq!(tagged.len(), 1);
    assert_eq!(database.updates().len(), 1);
    assert_eq!(
        database.deployment_info("d1"),
        Some(Json::Object(json_object(&[(
            "access_groups",
            list(&["group-a"]),
        )])))
    );
}

#[tokio::test]
async fn later_tag_failure() {
    let mut database = group_database(vec![
        deployment("d1", "alpha", None),
        deployment("d2", "beta", None),
    ]);
    database.fail_update_at = Some(1);
    let model_names = names(&["alpha", "beta"]);
    let error = tag_deployments(
        DeploymentSelection::ByNames(&model_names),
        "group-a",
        &database,
    )
    .await
    .expect_err("the later write failure");
    assert_eq!(value_error_message(error), "scripted update failure");
    let updated: Vec<String> = database
        .updates()
        .into_iter()
        .map(|update| update.0)
        .collect();
    assert_eq!(updated, names(&["d1"]));
    assert!(database.deployment_info("d2").is_none());
}

#[tokio::test]
async fn missing_deployment_is_400() {
    let database = group_database(Vec::new());
    let error = find_deployment_info_or_400("missing", &database)
        .await
        .expect_err("the missing row");
    let failure = http_failure(error);
    assert_eq!(failure.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        failure.message,
        "Deployment with model_id 'missing' not found in Database."
    );
}

#[tokio::test]
async fn absent_model_info_starts_empty() {
    let database = group_database(vec![deployment("d1", "alpha", None)]);
    let pair = update_deployment_group("d1", None, "group-a", GroupChange::Add, &database)
        .await
        .expect("the tag")
        .expect("a written pair");
    assert_eq!(
        pair.1,
        json_object(&[("access_groups", list(&["group-a"]))])
    );
}

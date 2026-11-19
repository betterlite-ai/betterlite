use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_cache::payload;
use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use tokio_util::task::TaskTracker;
use tracing::Level;

use super::super::{UpdateInput, update_spend_cache};
use super::support::{
    FakeCache, FakeState, RecordingAlerts, WriteCall, capture_logs, json_from, update_deps, written,
};

fn user_fixture() -> Json {
    json_from(r#"{"user_id":"user-1","spend":1}"#)
}

fn end_user_fixture() -> Json {
    json_from(r#"{"user_id":"end-1","blocked":false,"spend":2}"#)
}

fn team_fixture() -> Json {
    json_from(r#"{"team_id":"team-1","spend":3,"last_refreshed_at":null}"#)
}

fn tag_fixture() -> Json {
    json_from(r#"{"tag_name":"qa","models":[],"spend":4}"#)
}

fn entry<'write>(write: &'write WriteCall, key: &str) -> &'write Json {
    let found = write
        .entries
        .iter()
        .find(|(name, _value)| name == key)
        .expect("the queued entry");
    &found.1
}

fn decode<Model>(write: &WriteCall, key: &str) -> Model
where
    Model: DeserializeOwned,
{
    payload::decode::<Model>(entry(write, key))
        .expect("the object decodes")
        .expect("the object is present")
}

fn write(writes: &[WriteCall], local_only: bool) -> &WriteCall {
    writes
        .iter()
        .find(|write| write.local_only == local_only)
        .expect("the queued write")
}

fn group_fixtures() -> BTreeMap<String, Json> {
    BTreeMap::from([
        ("user-1".to_owned(), user_fixture()),
        (GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(), json_from("10")),
        ("end_user_id:end-1".to_owned(), end_user_fixture()),
        ("team_id:team-1".to_owned(), team_fixture()),
        ("tag:qa".to_owned(), tag_fixture()),
    ])
}

fn all_groups_state() -> FakeState {
    FakeState::new()
        .with_cached(group_fixtures())
        .with_management_ttl(33.0)
}

fn user_cost_input() -> UpdateInput {
    UpdateInput {
        token: None,
        user_id: Some("user-1".to_owned()),
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::from(5_i32)),
        span: None,
        tags: None,
    }
}

fn all_groups_input() -> UpdateInput {
    let mut input = user_cost_input();
    input.end_user_id = Some("end-1".to_owned());
    input.team_id = Some("team-1".to_owned());
    input.tags = Some(vec![Json::Text("qa".to_owned())]);
    input
}

async fn run_update(cache: &Arc<FakeCache>, input: &UpdateInput) {
    let tasks = TaskTracker::new();
    let alerts = Arc::new(RecordingAlerts::default());
    let deps = update_deps(Arc::clone(cache), alerts, &tasks);
    update_spend_cache(input, &deps)
        .await
        .expect("the update runs");
    tasks.close();
    tasks.wait().await;
}

fn assert_object_groups(objects: &WriteCall) {
    let queues: Vec<&str> = objects
        .entries
        .iter()
        .map(|(key, _value)| key.as_str())
        .collect();
    assert_eq!(
        queues,
        ["user-1", "end_user_id:end-1", "team_id:team-1", "tag:qa"]
    );
    let user: LiteLLMUserTable = decode(objects, "user-1");
    assert_eq!(user.spend, Decimal::from(6_i32));
    let end_user: LiteLLMEndUserTable = decode(objects, "end_user_id:end-1");
    assert_eq!(end_user.spend, Decimal::from(7_i32));
    let team: LiteLLMTeamTableCachedObj = decode(objects, "team_id:team-1");
    assert_eq!(team.team.fields.spend, Some(Decimal::from(8_i32)));
    let tag: TagTable = decode(objects, "tag:qa");
    assert_eq!(tag.spend, Decimal::from(9_i32));
}

#[tokio::test]
async fn every_object_group_adds_the_cost_and_the_global_spend_is_written_apart() {
    let cache = FakeCache::new(all_groups_state());
    run_update(&cache, &all_groups_input()).await;
    let state = cache.state();
    let writes = state.writes.clone();
    drop(state);
    assert_eq!(writes.len(), 2);
    let objects = write(&writes, true);
    let global = write(&writes, false);
    assert_eq!(objects.ttl.to_bits(), 33.0_f64.to_bits());
    assert_eq!(global.ttl.to_bits(), 33.0_f64.to_bits());
    assert_object_groups(objects);
    assert_eq!(
        entry(global, GLOBAL_PROXY_SPEND_CACHE_KEY),
        &json_from("15")
    );
}

#[tokio::test]
async fn a_missing_user_row_leaves_the_global_spend_alone() {
    let cache = FakeCache::new(FakeState::new().with_cached(BTreeMap::from([(
        GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(),
        json_from("10"),
    )])));
    run_update(&cache, &user_cost_input()).await;
    assert!(cache.state().writes.is_empty());
}

#[tokio::test]
async fn a_missing_global_spend_is_not_created() {
    let cache = FakeCache::new(
        FakeState::new().with_cached(BTreeMap::from([("user-1".to_owned(), user_fixture())])),
    );
    run_update(&cache, &user_cost_input()).await;
    let state = cache.state();
    assert_eq!(state.writes.len(), 1);
    assert!(state.writes[0].local_only);
    assert_eq!(state.writes[0].entries.len(), 1);
    assert_eq!(state.writes[0].entries[0].0, "user-1");
    drop(state);
}

#[tokio::test]
async fn a_text_global_spend_is_not_replaced() {
    let cache = FakeCache::new(FakeState::new().with_cached(BTreeMap::from([
        ("user-1".to_owned(), user_fixture()),
        (
            GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(),
            json_from(r#""10""#),
        ),
    ])));
    run_update(&cache, &user_cost_input()).await;
    let state = cache.state();
    assert_eq!(state.writes.len(), 1);
    assert!(state.writes[0].local_only);
    assert_eq!(state.writes[0].entries.len(), 1);
    drop(state);
}

#[tokio::test]
async fn a_boolean_global_spend_is_not_replaced() {
    let cache = FakeCache::new(FakeState::new().with_cached(BTreeMap::from([
        ("user-1".to_owned(), user_fixture()),
        (GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(), json_from("true")),
    ])));
    run_update(&cache, &user_cost_input()).await;
    let state = cache.state();
    assert_eq!(state.writes.len(), 1);
    assert!(state.writes[0].local_only);
    assert_eq!(state.writes[0].entries.len(), 1);
    drop(state);
}

#[tokio::test]
async fn objects_that_do_not_decode_are_skipped() {
    let undecodable = json_from(r#"{"spend":1}"#);
    let cache = FakeCache::new(FakeState::new().with_cached(BTreeMap::from([
        ("user-1".to_owned(), undecodable.clone()),
        (GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(), json_from("10")),
        ("end_user_id:end-1".to_owned(), undecodable.clone()),
        ("team_id:team-1".to_owned(), undecodable.clone()),
        ("tag:qa".to_owned(), undecodable),
    ])));
    run_update(&cache, &all_groups_input()).await;
    assert!(cache.state().writes.is_empty());
}

#[tokio::test]
async fn empty_and_non_text_tags_are_skipped() {
    let cache = FakeCache::new(
        FakeState::new().with_cached(BTreeMap::from([("tag:qa".to_owned(), tag_fixture())])),
    );
    let input = UpdateInput {
        token: None,
        user_id: None,
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::from(5_i32)),
        span: None,
        tags: Some(vec![
            Json::Text(String::new()),
            Json::Number(serde_json::Number::from(7_i32)),
            Json::Text("qa".to_owned()),
        ]),
    };
    run_update(&cache, &input).await;
    let state = cache.state();
    assert_eq!(state.writes.len(), 1);
    assert_eq!(state.writes[0].entries.len(), 1);
    let tag: TagTable = decode(&state.writes[0], "tag:qa");
    drop(state);
    assert_eq!(tag.spend, Decimal::from(9_i32));
}

#[tokio::test]
async fn a_null_tag_spend_is_skipped() {
    let cache = FakeCache::new(FakeState::new().with_cached(BTreeMap::from([(
        "tag:qa".to_owned(),
        json_from(r#"{"tag_name":"qa","models":[],"spend":null}"#),
    )])));
    let input = UpdateInput {
        token: None,
        user_id: None,
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::from(5_i32)),
        span: None,
        tags: Some(vec![Json::Text("qa".to_owned())]),
    };
    run_update(&cache, &input).await;
    assert!(cache.state().writes.is_empty());
}

#[tokio::test]
async fn no_user_id_leaves_the_global_spend_alone() {
    let cache = FakeCache::new(all_groups_state());
    let input = UpdateInput {
        token: None,
        user_id: None,
        end_user_id: Some("end-1".to_owned()),
        team_id: None,
        response_cost: Some(Decimal::from(5_i32)),
        span: None,
        tags: None,
    };
    run_update(&cache, &input).await;
    let state = cache.state();
    assert_eq!(state.reads, [vec!["end_user_id:end-1".to_owned()]]);
    assert_eq!(state.writes.len(), 1);
    assert!(state.writes[0].local_only);
    assert_eq!(state.writes[0].entries.len(), 1);
    assert_eq!(state.writes[0].entries[0].0, "end_user_id:end-1");
    drop(state);
}

#[tokio::test]
async fn a_request_without_a_cost_reads_and_updates_nothing() {
    let cache = FakeCache::new(all_groups_state());
    let mut input = all_groups_input();
    input.response_cost = None;
    run_update(&cache, &input).await;
    let state = cache.state();
    assert!(state.reads.is_empty());
    assert!(state.declared.is_empty());
    assert!(state.writes.is_empty());
    drop(state);
}

#[tokio::test]
async fn a_user_arithmetic_failure_warns_and_the_other_groups_still_run() {
    let cache = FakeCache::new(FakeState::new().with_cached(BTreeMap::from([
        ("user-1".to_owned(), user_fixture()),
        (GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned(), json_from("10")),
        (
            "tag:qa".to_owned(),
            json_from(r#"{"tag_name":"qa","models":[],"spend":-100}"#),
        ),
    ])));
    let input = UpdateInput {
        token: None,
        user_id: Some("user-1".to_owned()),
        end_user_id: None,
        team_id: None,
        response_cost: Some(Decimal::MAX),
        span: None,
        tags: Some(vec![Json::Text("qa".to_owned())]),
    };
    let (log, _guard) = capture_logs(Level::WARN);
    run_update(&cache, &input).await;
    let text = written(&log);
    assert!(text.contains("Spend tracking - failed to update user spend in cache."));
    let state = cache.state();
    assert_eq!(state.writes.len(), 1);
    assert!(state.writes[0].local_only);
    assert_eq!(state.writes[0].entries.len(), 1);
    assert!(
        state.writes[0]
            .entries
            .iter()
            .all(|(key, _value)| key == "tag:qa")
    );
    drop(state);
}

#[tokio::test]
async fn a_parent_otel_span_does_not_change_the_spend_update() {
    let cache = FakeCache::new(all_groups_state());
    let mut input = all_groups_input();
    input.span = Some(ParentOtelSpan::Opaque);
    run_update(&cache, &input).await;
    let state = cache.state();
    let writes = state.writes.clone();
    drop(state);
    assert_eq!(writes.len(), 2);
    assert_object_groups(write(&writes, true));
    assert_eq!(
        entry(write(&writes, false), GLOBAL_PROXY_SPEND_CACHE_KEY),
        &json_from("15")
    );
}

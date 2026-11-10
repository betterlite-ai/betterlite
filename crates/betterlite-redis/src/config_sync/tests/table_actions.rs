use std::sync::Arc;

use rstest::rstest;

use super::support::{
    Journal, RecordingTable, failing_publisher, recording_publisher, redis_cache,
};
use crate::config_sync::publish::redis_config_change_publisher;
use crate::config_sync::table_actions::{TableActions as _, wrap_table_actions_for_config_sync};
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test]
async fn create_publishes_after_a_successful_write() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_proxymodeltable",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.create("row").await, Ok(1));
    assert_eq!(
        journal.entries(),
        ["create:row", "publish:litellm_proxymodeltable"]
    );
}

#[tokio::test]
async fn create_many_publishes_after_a_successful_write() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_credentialstable",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.create_many(vec!["one", "two"]).await, Ok(2));
    assert_eq!(
        journal.entries(),
        ["create_many:2", "publish:litellm_credentialstable"]
    );
}

#[tokio::test]
async fn update_publishes_after_a_successful_write() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_policytable",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.update("new", "filter").await, Ok(Some(2)));
    assert_eq!(
        journal.entries(),
        ["update:new:filter", "publish:litellm_policytable"]
    );
}

#[tokio::test]
async fn a_zero_row_update_many_still_publishes() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_guardrailstable",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.update_many("new", "filter").await, Ok(0));
    assert_eq!(
        journal.entries(),
        ["update_many:new:filter", "publish:litellm_guardrailstable"]
    );
}

#[tokio::test]
async fn upsert_publishes_after_a_successful_write() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_configoverrides",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.upsert("filter", "new").await, Ok(3));
    assert_eq!(
        journal.entries(),
        ["upsert:filter:new", "publish:litellm_configoverrides"]
    );
}

#[tokio::test]
async fn delete_publishes_even_when_no_row_matched() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_prompttable",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.delete("filter").await, Ok(None));
    assert_eq!(
        journal.entries(),
        ["delete:filter", "publish:litellm_prompttable"]
    );
}

#[tokio::test]
async fn delete_many_publishes_after_a_successful_write() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_searchtoolstable",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.delete_many("filter").await, Ok(0));
    assert_eq!(
        journal.entries(),
        ["delete_many:filter", "publish:litellm_searchtoolstable"]
    );
}

#[tokio::test]
async fn a_failed_write_is_returned_without_a_publish() {
    let journal = Journal::default();
    let actions = RecordingTable {
        journal: journal.clone(),
        failure: Some("database down"),
    };
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_uisettings",
        recording_publisher(&journal),
    );
    assert_eq!(wrapped.create("row").await, Err("database down"));
    assert_eq!(journal.entries(), ["create:row"]);
}

#[tokio::test]
async fn a_publish_failure_is_returned_after_the_write() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_cacheconfig",
        failing_publisher(&journal, "publish down"),
    );
    assert_eq!(wrapped.create("row").await, Err("publish down"));
    assert_eq!(
        journal.entries(),
        ["create:row", "publish:litellm_cacheconfig"]
    );
}

#[rstest]
#[case::proxy_model("litellm_proxymodeltable")]
#[case::credentials("litellm_credentialstable")]
#[case::guardrails("litellm_guardrailstable")]
#[case::policy("litellm_policytable")]
#[case::policy_attachment("litellm_policyattachmenttable")]
#[case::managed_vector_stores("litellm_managedvectorstorestable")]
#[case::managed_vector_store_index("litellm_managedvectorstoreindextable")]
#[case::mcp_server("litellm_mcpservertable")]
#[case::agents("litellm_agentstable")]
#[case::prompt("litellm_prompttable")]
#[case::search_tools("litellm_searchtoolstable")]
#[case::sso_config("litellm_ssoconfig")]
#[case::cache_config("litellm_cacheconfig")]
#[case::config_overrides("litellm_configoverrides")]
#[case::ui_settings("litellm_uisettings")]
#[tokio::test]
async fn every_synced_table_publishes_its_own_name(#[case] table_name: &'static str) {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped =
        wrap_table_actions_for_config_sync(actions, table_name, recording_publisher(&journal));
    assert!(wrapped.sync.is_some());
    assert_eq!(wrapped.create("row").await, Ok(1));
    let publish = format!("publish:{table_name}");
    assert_eq!(journal.entries(), ["create:row", publish.as_str()]);
}

#[rstest]
#[case::spend_logs("litellm_spendlogs")]
#[case::team_table("litellm_teamtable")]
#[case::config("litellm_config")]
#[case::uppercase("LITELLM_PROXYMODELTABLE")]
#[tokio::test]
async fn a_table_outside_the_allowlist_has_no_publish_target(#[case] table_name: &'static str) {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped =
        wrap_table_actions_for_config_sync(actions, table_name, recording_publisher(&journal));
    assert!(wrapped.sync.is_none());
    assert_eq!(wrapped.create("row").await, Ok(1));
    assert_eq!(journal.entries(), ["create:row"]);
}

#[tokio::test]
async fn a_wrapped_write_publishes_the_table_name_over_redis() {
    let server = FakeServer::start().await;
    server.script("PUBLISH", Reply::Int(1));
    let cache = redis_cache(&server, None).await;
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_agentstable",
        redis_config_change_publisher(Some(Arc::new(cache))),
    );
    assert_eq!(wrapped.create("row").await, Ok(1));
    assert_eq!(
        server.commands_named("PUBLISH")[0].args,
        [
            "PUBLISH",
            "litellm_proxy.config_change",
            r#"{"object_type":"litellm_agentstable"}"#,
        ]
    );
}

#[tokio::test]
async fn a_wrapped_write_without_coordination_redis_still_succeeds() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_agentstable",
        redis_config_change_publisher(None),
    );
    assert_eq!(wrapped.create("row").await, Ok(1));
    assert_eq!(journal.entries(), ["create:row"]);
}

#[tokio::test]
async fn the_inner_actions_stay_reachable_without_publication() {
    let journal = Journal::default();
    let actions = RecordingTable::recording(&journal);
    let wrapped = wrap_table_actions_for_config_sync(
        actions,
        "litellm_proxymodeltable",
        recording_publisher(&journal),
    );
    let inner: &RecordingTable = wrapped.as_ref();
    assert_eq!(inner.create("row").await, Ok(1));
    assert_eq!(journal.entries(), ["create:row"]);
}

use super::support::{harness, harness_without_server};
use crate::redis_cache_values::error::RedisCacheValuesError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

fn entries() -> Vec<(String, &'static str)> {
    vec![("alpha".to_owned(), "first"), ("beta".to_owned(), "second")]
}

#[tokio::test]
async fn sends_one_pipeline_of_namespaced_sets_in_order() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), Some(60), None);
    harness
        .values
        .async_set_cache_pipeline(&entries(), Some(30.0_f64))
        .await
        .expect("the pipelined write");
    let sets = server.commands_named("SET");
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].args, ["SET", "ns:alpha", "\"first\"", "EX", "30"]);
    assert_eq!(sets[1].args, ["SET", "ns:beta", "\"second\"", "EX", "30"]);
    assert!(server.commands_named("MULTI").is_empty());
    assert_eq!(
        harness.hooks.entries(),
        vec!["success:async_set_cache_pipeline:None".to_owned()]
    );
}

#[tokio::test]
async fn an_empty_list_sends_nothing_and_opens_no_client() {
    let server = FakeServer::start().await;
    let harness = harness(&server, Some("ns"), Some(60), None);
    let empty: Vec<(String, &str)> = Vec::new();
    harness
        .values
        .async_set_cache_pipeline(&empty, Some(30.0_f64))
        .await
        .expect("the empty pipeline");
    assert!(server.commands().is_empty());
    assert_eq!(server.connections(), 0);
    assert_eq!(
        harness.gate.entries(),
        vec![
            "enter:async_set_cache_pipeline".to_owned(),
            "exit:1".to_owned()
        ]
    );
}

#[tokio::test]
async fn returns_a_client_failure() {
    let harness = harness_without_server(Some("ns"), Some(60));
    let error = harness
        .values
        .async_set_cache_pipeline(&entries(), None)
        .await
        .expect_err("the unreachable client fails");
    assert!(matches!(error, RedisCacheValuesError::ClientReuse(_)));
    assert!(harness.hooks.entries().is_empty());
    let gate = harness.gate.entries();
    assert_eq!(gate.len(), 2);
    assert_eq!(gate[0], "enter:async_set_cache_pipeline");
    assert!(gate[1].starts_with("failure:"));
}

#[tokio::test]
async fn swallows_an_execute_failure() {
    let server = FakeServer::start().await;
    server.script("SET", Reply::Error("ERR broken".to_owned()));
    let harness = harness(&server, Some("ns"), Some(60), None);
    harness
        .values
        .async_set_cache_pipeline(&entries(), None)
        .await
        .expect("the swallowed failure");
    let hooks = harness.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].starts_with("failure:async_set_cache_pipeline:"));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:async_set_cache_pipeline");
    assert!(gate[1].starts_with("swallowed:1:"));
    assert_eq!(gate[2], "exit:1");
}

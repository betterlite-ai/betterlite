use super::support::harness;
use crate::cache_primitives::protocol::RedisScalar;
use crate::redis_cache::error::RedisCacheError;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_registered_script_runs_under_the_guard() {
    let server = FakeServer::start().await;
    server.script("EVALSHA", Reply::Int(7));
    let harness = harness(&server, Some("ns"), 60).await;
    let script = harness.cache.counters.async_register_script("return 1");
    let result = harness
        .cache
        .run_script(
            &script,
            &["alpha".to_owned()],
            &[RedisScalar::Text("argument".to_owned())],
        )
        .await
        .expect("the script run");
    assert_eq!(result, redis::Value::Int(7));
    assert_eq!(
        server.commands_named("EVALSHA")[0].args,
        ["EVALSHA", script.sha1.as_str(), "1", "ns:alpha", "argument"]
    );
    assert_eq!(harness.gate.entries(), ["enter:run_script", "exit:1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_script_is_recorded_and_returned() {
    let server = FakeServer::start().await;
    server.script("EVALSHA", Reply::Error("ERR script failed".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    let script = harness.cache.counters.async_register_script("return 1");
    let error = harness
        .cache
        .run_script(&script, &["alpha".to_owned()], &[])
        .await
        .expect_err("the script error");
    assert!(matches!(error, RedisCacheError::Counters(_)));
    let gate = harness.gate.entries();
    assert_eq!(gate[0], "enter:run_script");
    assert!(gate[1].starts_with("failure:"));
    assert_eq!(gate.len(), 2);
}

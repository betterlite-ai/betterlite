use super::support::harness;
use crate::cache_primitives::protocol::RedisScalar;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_slice_reaches_the_fake_server_through_one_transport() {
    let server = FakeServer::start().await;
    server.script("DEL", Reply::Int(1));
    server.script("RPUSH", Reply::Int(1));
    server.script("EVAL", Reply::Bulk("2.5".to_owned()));
    let harness = harness(&server, Some("ns"), 60).await;
    harness
        .cache
        .values
        .async_set_cache(Some("alpha"), &1_i32, None, false)
        .await
        .expect("the cached write");
    harness
        .cache
        .counters
        .async_set_max("beta", 2.5_f64, None)
        .await
        .expect("the max write");
    harness
        .cache
        .admin
        .async_delete_cache("gamma")
        .await
        .expect("the delete");
    harness
        .cache
        .lists
        .async_rpush("delta", &[RedisScalar::Text("payload".to_owned())])
        .await
        .expect("the push");

    let sets = server.commands_named("SET");
    assert_eq!(sets[0].args, ["SET", "ns:alpha", "1", "EX", "60"]);
    let evals = server.commands_named("EVAL");
    assert!(evals[0].args.iter().any(|argument| argument == "ns:beta"));
    assert_eq!(server.commands_named("DEL")[0].args, ["DEL", "ns:gamma"]);
    assert_eq!(
        server.commands_named("RPUSH")[0].args,
        ["RPUSH", "ns:delta", "payload"]
    );
}

use std::sync::Arc;

use super::support::cache;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};
use crate::request_batch::batches::POST_CALL_FLUSH_DEADLINE;
use crate::request_batch::scope::{
    active_request_redis_batch, active_request_redis_batches, request_redis_batch_scope,
};

#[tokio::test]
async fn request_batch_nested_scopes_join_the_outer_batches() {
    let server = FakeServer::start().await;
    server.script("MGET", Reply::Array(vec![Reply::Nil]));
    let cached = cache(&server, Some("ns")).await;

    let (outer, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
        let outer_request = active_request_redis_batches().expect("the outer scope");
        let outer_batch = active_request_redis_batch(&cached).expect("the outer batch");

        let (nested, ()) = request_redis_batch_scope(POST_CALL_FLUSH_DEADLINE, async {
            let inner_request = active_request_redis_batches().expect("the outer scope inside");
            assert!(Arc::ptr_eq(&outer_request, &inner_request));
            let inner_batch = active_request_redis_batch(&cached).expect("the outer batch inside");
            assert!(Arc::ptr_eq(&outer_batch, &inner_batch));
            inner_batch
                .mget(&["a".to_owned()])
                .await
                .expect("the nested read");
        })
        .await;

        assert!(Arc::ptr_eq(&outer_request, &nested));
        assert!(Arc::ptr_eq(
            &outer_batch,
            &active_request_redis_batch(&cached).expect("the outer batch after")
        ));
        assert_eq!(
            active_request_redis_batches().map(|request| request.batches().len()),
            Some(1)
        );
    })
    .await;

    assert_eq!(outer.batches().len(), 1);
}

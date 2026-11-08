use std::time::{SystemTime, UNIX_EPOCH};

use rstest::rstest;

use super::fake_redis::FakeRedis;
use super::fixtures::{TIMESTAMP, command_args};
use crate::redis_semantic_cache::live;
use crate::redis_semantic_cache::ports::{CacheEntryFields, SemanticClock as _, SystemClock};

#[test]
fn store_system_clock_reports_seconds_since_the_epoch() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_secs_f64();
    let now = SystemClock.now_seconds().expect("a clock after the epoch");
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_secs_f64();
    assert!(before <= now && now <= after);
}

#[rstest]
#[case::with_ttl(Some(30), 1)]
#[case::without_ttl(None, 0)]
#[tokio::test]
async fn hash_is_written_and_expired_only_with_a_ttl(
    #[case] ttl: Option<i64>,
    #[case] expires: usize,
) {
    let server = FakeRedis::start().await;
    let fields = CacheEntryFields {
        entry_id: "e1".to_owned(),
        prompt: "hi".to_owned(),
        response: "{}".to_owned(),
        prompt_vector: vec![0, 0, 128, 63],
        inserted_at: TIMESTAMP,
        updated_at: TIMESTAMP,
        litellm_cache_key: "scope".to_owned(),
    };
    server
        .connect()
        .await
        .store_hash("idx:e1", &fields, ttl)
        .await
        .expect("stored");
    assert_eq!(
        server.commands("HSET"),
        [command_args(&live::store_command("idx:e1", &fields))]
    );
    assert_eq!(server.commands("EXPIRE").len(), expires);
    let order = server.arrival_order(&["HSET", "EXPIRE"]);
    assert_eq!(order, ["HSET", "EXPIRE"][..=expires]);
    if let Some(seconds) = ttl {
        assert_eq!(
            server.commands("EXPIRE"),
            [vec![
                b"EXPIRE".to_vec(),
                b"idx:e1".to_vec(),
                seconds.to_string().into_bytes()
            ]]
        );
    }
}

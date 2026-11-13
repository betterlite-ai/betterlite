use super::{Duration, StaticClient, advance, fetch_from_static, join};

#[tokio::test(start_paused = true)]
async fn timeout() {
    let slow = StaticClient {
        status: 200,
        body: br#"{"max_position_embeddings": 1}"#.to_vec(),
        delay: Some(Duration::from_secs(30)),
    };
    let (timed_out, ()) = join!(fetch_from_static(&slow), async {
        advance(Duration::from_secs(11)).await;
    });
    assert_eq!(timed_out, None);
}

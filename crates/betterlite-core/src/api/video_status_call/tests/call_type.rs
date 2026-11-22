use super::support::{Bench, Outcome, VIDEO_ID, async_call, call, handled, ready, request};

#[test]
fn call_type_status_sets_video_retrieve_before_the_handler() {
    let mut bench = Bench::new(Outcome::answers());
    ready(call(&mut bench, request(VIDEO_ID)));
    assert_eq!(bench.fixture.state.call_type, "video_retrieve");
    assert_eq!(handled(&bench.journal).logged.call_type, "video_retrieve");
}

#[tokio::test]
async fn call_type_status_async_call_sets_video_retrieve() {
    let mut bench = Bench::new(Outcome::answers());
    async_call(&mut bench, request(VIDEO_ID))
        .await
        .expect("the call answers");
    assert_eq!(bench.fixture.state.call_type, "video_retrieve");
    assert_eq!(handled(&bench.journal).logged.call_type, "video_retrieve");
}

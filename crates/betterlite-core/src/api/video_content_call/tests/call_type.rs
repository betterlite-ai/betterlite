use super::support::{Bench, Outcome, VIDEO_ID, call, handled, ready, request};

#[test]
fn call_type_content_leaves_the_logging_call_type_unchanged() {
    let mut bench = Bench::new(Outcome::Bytes(b"video".to_vec()));
    ready(call(&mut bench, request(VIDEO_ID)));
    assert_eq!(bench.fixture.state.call_type, "video_generation");
    assert_eq!(handled(&bench.journal).logged.call_type, "video_generation");
}

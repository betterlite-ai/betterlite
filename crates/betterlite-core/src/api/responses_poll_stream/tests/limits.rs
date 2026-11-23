use super::frame_selection::assert_failed_message;
use super::support::{self, Answer, AnswerStep, Scenario};
use crate::api::api_boundaries::ApiLimits;

const ITEM_ADDED: &str = r#"{"item":{"content":[],"id":"m1"},"type":"response.output_item.added"}"#;
const PART_ADDED: &str = r#"{"item_id":"m1","part":{"text":"","type":"output_text"},"type":"response.content_part.added"}"#;

async fn run_with_limits(events: &[&str], limits: ApiLimits) -> Scenario {
    let chunks = support::frames(events);
    let steps: Vec<AnswerStep> = chunks.into_iter().map(AnswerStep::Chunk).collect();
    let mut scenario = support::scenario(&[1000], Answer::Streaming(steps)).with_limits(limits);
    scenario
        .run()
        .await
        .expect("the worker records its outcome");
    scenario
}

#[tokio::test]
async fn items_over_the_bound_fail_the_record() {
    let limits = ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 2,
    };
    let scenario = run_with_limits(
        &[
            r#"{"item":{"content":[],"id":"m1"},"type":"response.output_item.added"}"#,
            r#"{"item":{"content":[],"id":"m2"},"type":"response.output_item.added"}"#,
            r#"{"item":{"content":[],"id":"m3"},"type":"response.output_item.added"}"#,
        ],
        limits,
    )
    .await;
    assert_failed_message(&scenario, "the request collected 3 items; the maximum is 2");
}

#[tokio::test]
async fn text_over_the_bound_fails_the_record() {
    let limits = ApiLimits {
        max_json_bytes: 512,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let delta = format!(
        r#"{{"content_index":0,"delta":"{}","item_id":"m1","type":"response.output_text.delta"}}"#,
        "x".repeat(600)
    );
    let scenario = run_with_limits(&[ITEM_ADDED, PART_ADDED, delta.as_str()], limits).await;
    assert_failed_message(
        &scenario,
        "the request JSON body is 600 bytes; the maximum is 512",
    );
}

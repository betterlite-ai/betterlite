use rstest::rstest;

use crate::posthog_logging::event::create_event_payload;
use crate::posthog_logging::tests::support::{fixed_id, payload};

fn event_name(call_type: &str) -> String {
    let mut payload = payload();
    payload.call_type = call_type.to_owned();
    create_event_payload(&payload, fixed_id)
        .expect("the event builds")
        .event
}

#[rstest]
#[case::completion("completion", "$ai_generation")]
#[case::async_completion("acompletion", "$ai_generation")]
#[case::embedding("embedding", "$ai_embedding")]
#[case::async_embedding("aembedding", "$ai_generation")]
#[case::capitalized("Embedding", "$ai_generation")]
#[case::empty("", "$ai_generation")]
#[case::image_generation("image_generation", "$ai_generation")]
fn posthog_event_type_follows_the_call_type(#[case] call_type: &str, #[case] expected: &str) {
    assert_eq!(event_name(call_type), expected);
}

#[test]
fn posthog_event_type_keeps_the_embedding_projection() {
    let mut payload = payload();
    payload.call_type = "embedding".to_owned();
    let event = create_event_payload(&payload, fixed_id).expect("the event builds");
    assert!(event.properties.contains_key("$ai_input"));
    assert!(!event.properties.contains_key("$ai_output_choices"));
    assert!(!event.properties.contains_key("$ai_output_tokens"));
}

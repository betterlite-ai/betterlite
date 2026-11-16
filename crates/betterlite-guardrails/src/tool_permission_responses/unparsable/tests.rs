#![cfg(test)]

use tokio::sync::mpsc;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::StreamItem;
use crate::tool_permission_responses::buffer_overflow::StreamBufferLimits;
use crate::tool_permission_responses::error::ResponseToolError;
use crate::tool_permission_responses::streaming::streaming_iterator_hook;
use crate::tool_permission_responses::tests::support::{GUARDRAIL_NAME, guardrail, services};

use super::UNPARSABLE_STREAM_MESSAGE;

#[test]
fn unparsable_stream_message_matches_litellm() {
    assert_eq!(
        UNPARSABLE_STREAM_MESSAGE,
        "Streamed response could not be verified for tool permissions (not a parseable Anthropic SSE stream), blocking it"
    );
}

#[tokio::test]
async fn an_unparsable_raw_stream_raises_a_400_failure_naming_the_guardrail() {
    let (sender, receiver) = mpsc::channel(1);
    sender
        .send(StreamItem::Bytes(b"data: {not json\n\n".to_vec()))
        .await
        .expect("send");
    drop(sender);
    let (output, mut output_receiver) = mpsc::channel(4);
    let error = streaming_iterator_hook(
        &guardrail(Vec::new()),
        StreamBufferLimits::default(),
        &services(),
        receiver,
        output,
    )
    .await
    .expect_err("an unparsable raw stream is blocked");
    let ResponseToolError::Guardrail(GuardrailFailure::GuardrailRaised {
        guardrail_name,
        message,
        wrap_default_message,
        status_code,
        blocked_content,
    }) = error
    else {
        panic!("expected a raised guardrail failure");
    };
    assert_eq!(guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert_eq!(message, UNPARSABLE_STREAM_MESSAGE);
    assert!(wrap_default_message);
    assert_eq!(status_code, 400);
    assert!(!blocked_content);
    assert!(output_receiver.try_recv().is_err());
}

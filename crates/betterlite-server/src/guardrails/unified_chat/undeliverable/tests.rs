#![cfg(test)]

use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;

use super::from_stream_sync;
use crate::guardrails::anthropic_translation::process_stream::error::ProcessStreamError;
use crate::guardrails::responses_translation::stream_sync::error::StreamSyncError;
use crate::guardrails::unified_chat::anthropic::AnthropicTranslationError;

fn sync_error(name: &str, reason: &str) -> StreamSyncError {
    StreamSyncError::Undeliverable {
        guardrail_name: name.to_owned(),
        reason: reason.to_owned(),
    }
}

#[test]
fn stream_sync_error_keeps_name_and_reason() {
    let converted = from_stream_sync(sync_error("guard", "the stream carries two choices"));
    assert_eq!(
        converted,
        UndeliverableStreamRewrite {
            guardrail_name: String::from("guard"),
            reason: String::from("the stream carries two choices"),
        }
    );
}

#[test]
fn stream_sync_error_without_name_uses_unknown() {
    let converted = from_stream_sync(sync_error("", "the stream carries two choices"));
    assert_eq!(converted.guardrail_name, "unknown");
    assert_eq!(converted.reason, "the stream carries two choices");
}

#[test]
fn stream_sync_error_keeps_the_unknown_name_b3672_wrote() {
    let converted = from_stream_sync(sync_error("unknown", "the stream carries two choices"));
    assert_eq!(converted.guardrail_name, "unknown");
}

#[test]
fn process_stream_error_undeliverable_passes_through() {
    let original = UndeliverableStreamRewrite {
        guardrail_name: String::from("guard"),
        reason: String::from("arguments need a single choice"),
    };
    let error =
        AnthropicTranslationError::from(ProcessStreamError::Undeliverable(original.clone()));
    match error {
        AnthropicTranslationError::Stream(ProcessStreamError::Undeliverable(inner)) => {
            assert_eq!(inner, original);
        }
        other => panic!("expected the undeliverable error unchanged, got {other:?}"),
    }
}

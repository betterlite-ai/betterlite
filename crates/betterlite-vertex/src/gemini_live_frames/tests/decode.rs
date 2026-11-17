#![cfg(test)]

use betterlite_core::api::realtime_contracts::transport::RealtimeMessage;
use rstest::rstest;

use crate::gemini_live_content::error::ContentError;
use crate::gemini_live_frames::decode::{decode_gemini_live_frame, decode_gemini_live_frame_bytes};
use crate::gemini_live_frames::error::LiveFrameError;
use crate::gemini_live_frames::state::CallerState;

use super::support::{LIVE_MODEL, Live};

#[test]
fn invalid_syntax_reports_the_frame_as_received() {
    let message = RealtimeMessage::Text("{not json".to_owned());
    let error = decode_gemini_live_frame(&message, usize::MAX).expect_err("the frame is not JSON");
    assert!(
        matches!(error, LiveFrameError::InvalidJson { message: reported, .. } if reported == "{not json")
    );
}

#[rstest]
#[case::list("[]")]
#[case::null("null")]
#[case::text("\"frame\"")]
#[case::number("5")]
fn non_object_frames_keep_their_parse_failure(#[case] source: &str) {
    let error = decode_gemini_live_frame_bytes(source.as_bytes(), usize::MAX)
        .expect_err("a non-object frame is refused");
    assert!(matches!(error, LiveFrameError::NotAnObject(_)));
}

#[test]
fn a_binary_frame_decodes_like_the_text_frame() {
    let text =
        decode_gemini_live_frame_bytes(br#"{"serverContent": {"turnComplete": true}}"#, usize::MAX)
            .expect("the frame decodes");
    let binary = decode_gemini_live_frame(&RealtimeMessage::Binary(text_bytes()), usize::MAX)
        .expect("the binary frame decodes");
    assert_eq!(text, binary);
}

fn text_bytes() -> Vec<u8> {
    br#"{"serverContent": {"turnComplete": true}}"#.to_vec()
}

#[test]
fn a_frame_over_the_limit_is_refused_before_parsing() {
    let error = decode_gemini_live_frame_bytes(b"{}", 1).expect_err("the frame is too large");
    assert!(matches!(error, LiveFrameError::TooLarge { limit: 1 }));
    let at_limit =
        decode_gemini_live_frame_bytes(b"{}", 2).expect("the frame at the limit decodes");
    assert_eq!(at_limit.entries.len(), 0);
}

#[test]
fn a_wrong_typed_field_is_refused_as_a_field() {
    let error = decode_gemini_live_frame_bytes(br#"{"usageMetadata": 5}"#, usize::MAX)
        .expect_err("a number is not usage metadata");
    assert!(matches!(
        &error,
        LiveFrameError::InvalidField { path, .. } if path == "usageMetadata"
    ));
}

#[test]
fn a_binary_frame_that_is_not_utf8_names_the_failure() {
    let error = decode_gemini_live_frame_bytes(b"{\"a\": \"\xff\"}", usize::MAX)
        .expect_err("the frame is not UTF-8");
    assert!(matches!(
        &error,
        LiveFrameError::InvalidJson { message, .. } if message.starts_with("the frame is not UTF-8")
    ));
}

#[test]
fn a_frame_nested_past_the_depth_cap_is_refused() {
    let source = format!("{}null{}", "{\"a\":".repeat(200), "}".repeat(200));
    let error = decode_gemini_live_frame_bytes(source.as_bytes(), usize::MAX)
        .expect_err("the frame nests past the depth cap");
    assert!(matches!(error, LiveFrameError::InvalidJson { .. }));
}

#[test]
fn a_null_server_content_stays_distinct_from_an_absent_one() {
    let absent = decode_gemini_live_frame_bytes(b"{}", usize::MAX).expect("the frame decodes");
    let null = decode_gemini_live_frame_bytes(br#"{"serverContent": null}"#, usize::MAX)
        .expect("the frame decodes");
    assert_eq!(absent.entries.len(), 0);
    assert!(null.entries.contains_key("serverContent"));
    assert_eq!(absent.message.server_content, null.message.server_content);
    let mut live = Live::new(LIVE_MODEL);
    let events = live.frame("{}").expect("an empty frame is known");
    assert!(events.is_empty());
    assert_eq!(live.caller, CallerState::default());
    let error = live
        .frame(r#"{"serverContent": null}"#)
        .expect_err("a null serverContent maps to no event");
    assert!(matches!(
        error,
        LiveFrameError::Content(ContentError::UnknownEvent(_))
    ));
}

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
    assert!(matches!(error, LiveFrameError::InvalidJson { message, .. } if message == "{not json"));
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

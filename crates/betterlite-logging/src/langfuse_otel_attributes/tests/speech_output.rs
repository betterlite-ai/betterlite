#![cfg(test)]

use super::output_content::get_output_content_by_type;
use crate::langfuse_projections::content::ResponseObj;

#[test]
fn speech_response_returns_speech_output() {
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::Speech), None),
        "speech-output"
    );
}

#[test]
fn a_non_embedding_call_type_keeps_speech_output() {
    assert_eq!(
        get_output_content_by_type(Some(&ResponseObj::Speech), Some("speech")),
        "speech-output"
    );
}

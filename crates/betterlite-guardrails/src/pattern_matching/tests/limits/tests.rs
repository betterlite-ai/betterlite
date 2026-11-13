#![cfg(test)]

use super::{ContentFilterAction, ContentFilterPattern, compile};
use crate::pattern_matching::{PatternError, blocked_words, masking, spans::MAX_TEXT_BYTES};
use rstest::rstest;

#[rstest]
#[case::text_limit("text")]
#[case::yaml_limit("yaml")]
#[case::format_limit("format")]
#[case::invalid_format("invalid_format")]
fn pattern_matching_behavior(#[case] input: &str) {
    match input {
        "text" => {
            let pattern = compile(&ContentFilterPattern {
                pattern_type: "regex".to_owned(),
                pattern_name: None,
                pattern: Some("x".to_owned()),
                name: None,
                action: ContentFilterAction::Mask,
            })
            .unwrap();
            assert!(matches!(
                pattern.find_spans(&"x".repeat(MAX_TEXT_BYTES.saturating_add(1))),
                Err(PatternError::TooLarge)
            ));
        }
        "yaml" => assert!(matches!(
            blocked_words::decode(&vec![b'x'; MAX_TEXT_BYTES.saturating_add(1)]),
            Err(blocked_words::WordsError::TooLarge)
        )),
        "format" => {
            masking::redaction_tag("{pattern_name:999999999999}", "x").unwrap_err();
        }
        _ => {
            masking::redaction_tag("{unknown}", "x").unwrap_err();
        }
    }
}

#[path = "compile/tests.rs"]
mod compile_limits;
#[path = "masking/tests.rs"]
mod mask_limits;
#[path = "redaction/tests.rs"]
mod redaction;
#[path = "text/tests.rs"]
mod text;
#[path = "yaml/tests.rs"]
mod yaml;

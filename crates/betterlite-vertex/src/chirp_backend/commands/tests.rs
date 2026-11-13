#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{ChirpCommand, ChirpEvent, decode_chirp_command};

#[test]
fn decodes_a_configure_command() {
    let command = decode_chirp_command(
        br#"{"kind":"configure","model":"chirp_2","language_codes":["en-US","fr-FR"],"sample_rate_hertz":16000}"#,
    )
    .expect("a valid configure command");
    assert_eq!(
        command,
        ChirpCommand::Configure {
            model: "chirp_2".to_owned(),
            language_codes: vec!["en-US".to_owned(), "fr-FR".to_owned()],
            sample_rate_hertz: 16000,
        }
    );
}

#[rstest]
#[case::finish_turn(br#"{"kind":"finish_turn"}"#, ChirpCommand::FinishTurn {})]
#[case::discard_turn(br#"{"kind":"discard_turn"}"#, ChirpCommand::DiscardTurn {})]
fn decodes_a_unit_command(#[case] message: &[u8], #[case] expected: ChirpCommand) {
    assert_eq!(
        decode_chirp_command(message).expect("a valid command"),
        expected
    );
}

#[rstest]
#[case::unknown_kind(br#"{"kind":"pause_turn"}"#)]
#[case::missing_kind(
    br#"{"model":"chirp_2","language_codes":["en-US"],"sample_rate_hertz":16000}"#
)]
#[case::missing_model(
    br#"{"kind":"configure","language_codes":["en-US"],"sample_rate_hertz":16000}"#
)]
#[case::wrong_type(br#"{"kind":"configure","model":"chirp_2","language_codes":["en-US"],"sample_rate_hertz":"16000"}"#)]
#[case::not_an_object(br#""configure""#)]
#[case::trailing_data(br#"{"kind":"finish_turn"} {"kind":"finish_turn"}"#)]
#[case::empty(b"")]
fn rejects_an_invalid_command(#[case] message: &[u8]) {
    assert!(decode_chirp_command(message).is_err());
}

#[test]
fn rejects_a_command_that_is_not_utf8() {
    assert!(decode_chirp_command(&[0x7b, 0xff, 0x7d]).is_err());
}

#[test]
fn serializes_the_control_events() {
    let configured = serde_json::to_string(&ChirpEvent::Configured).expect("a configured event");
    assert_eq!(configured, r#"{"kind":"configured"}"#);
    let finished = serde_json::to_string(&ChirpEvent::TurnFinished).expect("a turn event");
    assert_eq!(finished, r#"{"kind":"turn_finished"}"#);
    let discarded = serde_json::to_string(&ChirpEvent::TurnDiscarded {
        billed_seconds: 1.25,
    })
    .expect("a discard event");
    assert_eq!(
        discarded,
        r#"{"kind":"turn_discarded","billed_seconds":1.25}"#
    );
}

#[rstest]
fn replays_the_chirp_command_corpus(#[files("fuzz/corpus/chirp_command/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("the corpus file is readable");
    drop(decode_chirp_command(&bytes));
}

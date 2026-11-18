use super::*;
use crate::api::realtime_sender::error::RealtimeSenderError;
use futures_util::FutureExt as _;
use rstest::rstest;
use std::path::PathBuf;
use std::{fs, str};

#[test]
fn realtime_authorized_transcription_model() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let flat = sender.enforce_session_model(FLAT_UPDATE).unwrap();
    assert_eq!(parsed(&flat), parsed(FLAT_EXPECTED));
    let ga = sender.enforce_session_model(GA_UPDATE).unwrap();
    assert_eq!(parsed(&ga), parsed(GA_EXPECTED));
    let both = sender.enforce_session_model(BOTH_UPDATE).unwrap();
    assert_eq!(parsed(&both), parsed(BOTH_EXPECTED));
}

#[test]
fn enforce_marks_a_transcription_session() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    state.is_transcription_session.0 = false;
    let callbacks = no_guardrails();
    let ordinary = r#"{"type":"session.update","session":{"type":"realtime","input_audio_transcription":{"model":"allowed"}}}"#;
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.enforce_session_model(ordinary).unwrap()
    };
    assert!(!state.is_transcription_session.0);
    let transcription = r#"{"type":"transcription_session.update","session":{"type":"transcription","input_audio_transcription":{"model":"allowed"}}}"#;
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.enforce_session_model(transcription).unwrap()
    };
    assert!(state.is_transcription_session.0);
}

#[test]
fn enforce_keeps_original_bytes_for_unrelated_frames() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let unrelated = r#"{"type":"input_audio_buffer.append","audio":"AAAA"}"#;
    assert_eq!(sender.enforce_session_model(unrelated).unwrap(), unrelated);
    let append_with_session = r#"{"type":"input_audio_buffer.append","session":{"input_audio_transcription":{"model":"forbidden"}}}"#;
    assert_eq!(
        sender.enforce_session_model(append_with_session).unwrap(),
        append_with_session
    );
    let malformed = r#"{"type":"session.update""#;
    assert_eq!(sender.enforce_session_model(malformed).unwrap(), malformed);
}

#[test]
fn enforce_keeps_bytes_when_a_session_update_needs_no_rewrite() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let unchanged = r#"{"type": "session.update", "session": {"type": "realtime", "input_audio_transcription": {"model": "allowed"}}}"#;
    assert_eq!(sender.enforce_session_model(unchanged).unwrap(), unchanged);
}

#[test]
fn enforce_ignores_missing_and_nonobjects() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let no_session = r#"{"type":"session.update"}"#;
    assert_eq!(
        sender.enforce_session_model(no_session).unwrap(),
        no_session
    );
    let session_not_object = r#"{"type":"session.update","session":"none"}"#;
    assert_eq!(
        sender.enforce_session_model(session_not_object).unwrap(),
        session_not_object
    );
    let no_transcription = r#"{"type":"session.update","session":{"audio":"flat","input_audio_transcription":"flat"}}"#;
    assert_eq!(
        sender.enforce_session_model(no_transcription).unwrap(),
        no_transcription
    );
    let top_level = r#"{"type":"session.update","session":{"model":"client-model","input_audio_transcription":{"model":"forbidden"}}}"#;
    let expected = r#"{"type":"session.update","session":{"model":"client-model","input_audio_transcription":{"model":"allowed"}}}"#;
    assert_eq!(
        parsed(&sender.enforce_session_model(top_level).unwrap()),
        parsed(expected)
    );
}

#[test]
fn enforce_does_nothing_without_a_forced_model() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let update = r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"}}}"#;
    assert_eq!(sender.enforce_session_model(update).unwrap(), update);
}

#[test]
fn enforce_rejects_a_json_nonobject() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let result = sender.enforce_session_model("[1,2]");
    assert!(matches!(
        result,
        Err(RealtimeSenderError::SessionUpdateNotObject)
    ));
    let null_result = sender.enforce_session_model("null");
    assert!(matches!(
        null_result,
        Err(RealtimeSenderError::SessionUpdateNotObject)
    ));
}

#[test]
fn enforce_rejects_session_update_past_nesting_limit() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let nested = format!("{}null{}", "[".repeat(64), "]".repeat(64));
    let deep = format!("{{\"type\":\"session.update\",\"session\":{{\"extension\":{nested}}}}}");
    let result = sender.enforce_session_model(&deep);
    assert!(matches!(
        result,
        Err(RealtimeSenderError::SessionUpdateTooDeep)
    ));
}

#[rstest]
#[case::inside(61, true)]
#[case::at_limit(62, true)]
#[case::outside(63, false)]
fn enforce_nesting_boundary(#[case] arrays: usize, #[case] accepted: bool) {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let nested = format!("{}null{}", "[".repeat(arrays), "]".repeat(arrays));
    let message = format!(r#"{{"type":"session.update","session":{{"extension":{nested}}}}}"#);
    let result = sender.enforce_session_model(&message);
    if accepted {
        assert_eq!(result.unwrap(), message);
    } else {
        assert!(matches!(
            result,
            Err(RealtimeSenderError::SessionUpdateTooDeep)
        ));
    }
}

#[test]
fn enforce_ignores_quoted_nesting() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let extension = serde_json::to_string(&"\"[{\\".repeat(70)).unwrap();
    let message = format!(r#"{{"type":"session.update","session":{{"extension":{extension}}}}}"#);
    assert_eq!(sender.enforce_session_model(&message).unwrap(), message);
}

#[rstest]
#[case::escaped_quote("\"")]
#[case::escaped_backslash("\\")]
#[case::quote_then_backslash("\"\\")]
fn enforce_depth_after_escapes(#[case] text: &str) {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let prefix = serde_json::to_string(text).unwrap();
    let nested = format!("{}null{}", "[".repeat(63), "]".repeat(63));
    let message = format!(
        r#"{{"type":"session.update","session":{{"prefix":{prefix},"extension":{nested}}}}}"#
    );
    assert!(matches!(
        sender.enforce_session_model(&message),
        Err(RealtimeSenderError::SessionUpdateTooDeep)
    ));
}

#[test]
fn enforce_counts_sibling_depths() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let siblings = "[],".repeat(70);
    let message =
        format!(r#"{{"type":"session.update","session":{{"extension":[{siblings}null]}}}}"#);
    assert_eq!(sender.enforce_session_model(&message).unwrap(), message);
}

#[rstest]
#[case::envelope(r#"{"type":"session.update","type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"}}}"#)]
#[case::session(r#"{"type":"session.update","session":{},"session":{"input_audio_transcription":{"model":"forbidden"}}}"#)]
#[case::flat_model(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"allowed","model":"forbidden"}}}"#)]
#[case::ga_model(r#"{"type":"session.update","session":{"audio":{"input":{"transcription":{"model":"allowed","model":"forbidden"}}}}}"#)]
fn enforce_rejects_duplicate_modeled_keys(#[case] message: &str) {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(matches!(
        sender.enforce_session_model(message),
        Err(RealtimeSenderError::SessionUpdateInvalid(_))
    ));
}

#[rstest]
fn replays_the_realtime_enforcement_corpus(
    #[files("fuzz/corpus/realtime_enforcement/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(message) = str::from_utf8(&bytes) else {
        return;
    };
    let mut transcription = false;
    let _result = super::super::enforce::enforce_session_update::<Infallible, Infallible>(
        message,
        "allowed",
        &mut transcription,
    );
}

#[rstest]
#[case::lone_high_surrogate(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"note":"\ud800"}}"#)]
#[case::lone_low_surrogate(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"note":"\udc00"}}"#)]
#[case::surrogate_key(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"\ud800":1}}"#)]
#[case::not_a_number(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature":NaN}}"#)]
#[case::infinity(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature":Infinity}}"#)]
#[case::negative_infinity(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature":-Infinity}}"#)]
#[case::not_a_number_after_space(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature": NaN}}"#)]
#[case::not_a_number_with_space(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature":NaN }}"#)]
#[case::not_a_number_in_array(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"values":[NaN]}}"#)]
#[case::negative_infinity_after_comma(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"values":[0,-Infinity]}}"#)]
#[case::transcription_session(r#"{"type":"transcription_session.update","session":{"type":"transcription","input_audio_transcription":{"model":"forbidden"},"note":"\ud800"}}"#)]
fn enforce_rejects_a_frame_python_decodes(#[case] message: &str) {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(matches!(
        sender.send_to_backend(message).now_or_never().unwrap(),
        Err(RealtimeSenderError::SessionUpdateUnreadable)
    ));
    assert!(!state.is_transcription_session.0);
    assert!(fixture.script().seen.is_empty());
    assert!(fixture.backend_log.sent().is_empty());
}

#[test]
fn enforce_rewrites_a_frame_with_a_surrogate_pair() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let pair = format!(r"\u{:04x}\u{:04x}", 0xD83D_u32, 0xDE00_u32);
    let message = format!(
        r#"{{"type":"session.update","session":{{"input_audio_transcription":{{"model":"forbidden"}}}},"note":"{pair}"}}"#
    );
    let expected = format!(
        r#"{{"type":"session.update","session":{{"input_audio_transcription":{{"model":"allowed"}}}},"note":"{pair}"}}"#
    );
    assert_eq!(
        parsed(&sender.enforce_session_model(&message).unwrap()),
        parsed(&expected)
    );
}

#[rstest]
#[case::truncated(r#"{"type":"session.update","session":{"note":"\ud800"}"#)]
#[case::trailing_comma(r#"{"type":"session.update","session":{"note":"\ud800",}}"#)]
#[case::unquoted_key("{NaN:1}")]
#[case::number_then_constant(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature":1NaN}}"#)]
#[case::constant_with_fraction(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature":NaN.0}}"#)]
#[case::constant_then_digit(r#"{"type":"session.update","session":{"input_audio_transcription":{"model":"forbidden"},"temperature":Infinity0}}"#)]
#[case::invalid_escape(r#"{"type":"session.update","session":{"note":"\q"}}"#)]
fn enforce_keeps_a_frame_python_cannot_decode(#[case] message: &str) {
    let mut transcription = false;
    let unchanged = super::super::enforce::enforce_session_update::<Infallible, Infallible>(
        message,
        "allowed",
        &mut transcription,
    )
    .unwrap();
    assert_eq!(unchanged, message);
    assert!(!transcription);
}

#[test]
fn enforce_rejects_a_python_only_json_nonobject() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    assert!(matches!(
        sender.enforce_session_model(r#""\ud800""#),
        Err(RealtimeSenderError::SessionUpdateNotObject)
    ));
    assert!(matches!(
        sender.enforce_session_model("NaN"),
        Err(RealtimeSenderError::SessionUpdateNotObject)
    ));
}

#[rstest]
#[case::null("null")]
#[case::boolean("true")]
#[case::integer("42")]
#[case::decimal("1.25")]
#[case::exponent("1e40")]
#[case::large_integer("18446744073709551616")]
#[case::text(r#""not-an-object""#)]
#[case::array(r#"[{"model":"forbidden"}]"#)]
fn enforce_preserves_nonobject_transcription(#[case] value: &str) {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let message = format!(
        r#"{{"type":"session.update","session":{{"input_audio_transcription":{value},"audio":{{"input":{{"transcription":{value}}}}}}}}}"#
    );
    assert_eq!(sender.enforce_session_model(&message).unwrap(), message);
}

#[rstest]
#[case::session("1.25")]
#[case::session_type(r#"{"type":1.25,"input_audio_transcription":{"model":"forbidden"}}"#)]
#[case::audio(r#"{"audio":1.25,"input_audio_transcription":{"model":"forbidden"}}"#)]
#[case::audio_input(
    r#"{"audio":{"input":1.25},"input_audio_transcription":{"model":"forbidden"}}"#
)]
#[case::flat_transcription(r#"{"input_audio_transcription":1.25,"audio":{"input":{"transcription":{"model":"forbidden"}}}}"#)]
#[case::ga_transcription(r#"{"input_audio_transcription":{"model":"forbidden"},"audio":{"input":{"transcription":1.25}}}"#)]
#[case::large_sibling(r#"{"input_audio_transcription":{"model":"forbidden"},"audio":{"input":{"transcription":18446744073709551616}}}"#)]
fn enforce_preserves_numbers_when_rewriting(#[case] session: &str) {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let message = format!(r#"{{"type":"session.update","session":{session}}}"#);
    let expected = message.replace("forbidden", "allowed");
    let enforced = sender.enforce_session_model(&message).unwrap();
    assert_eq!(parsed(&enforced), parsed(&expected));
    assert!(!state.is_transcription_session.0);
}

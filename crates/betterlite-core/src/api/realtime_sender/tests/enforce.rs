use super::*;
use crate::api::realtime_sender::error::RealtimeSenderError;
use rstest::rstest;

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
        sender.enforce_session_model(ordinary).unwrap();
    }
    assert!(!state.is_transcription_session.0);
    let transcription = r#"{"type":"transcription_session.update","session":{"type":"transcription","input_audio_transcription":{"model":"allowed"}}}"#;
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.enforce_session_model(transcription).unwrap();
    }
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
    let result = sender.enforce_session_model("null");
    assert!(matches!(
        result,
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

use super::*;
use crate::api::realtime_sender::error::RealtimeSenderError;

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
    let malformed = r#"{"type":"session.update""#;
    assert_eq!(sender.enforce_session_model(malformed).unwrap(), malformed);
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

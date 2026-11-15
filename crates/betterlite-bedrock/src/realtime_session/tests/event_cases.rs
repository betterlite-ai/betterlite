use super::support::{
    CREATED, FixedIds, MODEL_EVENT, NO_MODEL, TRACE_ID, UPDATED, UPDATED_EMPTY, UPDATED_EXPLICIT,
};
use crate::invoke_primitives::environment::error_class;
use crate::realtime_session::records::{SessionEventType, SessionObject};
use crate::realtime_session::{
    REQUIRES_SESSION_CONFIGURATION, RealtimeSession, complete_url, validate_environment,
};

pub(super) fn created_updated_events() {
    created_event();
    updated_event();
    session_event_helpers();
}

fn created_event() {
    let ids = FixedIds::new(&[CREATED, NO_MODEL]);
    let created = RealtimeSession::session_event(
        SessionEventType::Created,
        Some("model-a"),
        TRACE_ID,
        None,
        &ids,
    );
    assert_eq!(created.event_type, SessionEventType::Created);
    assert_eq!(created.event_id, CREATED);
    assert_eq!(
        created.session,
        SessionObject {
            id: TRACE_ID.to_owned(),
            modalities: vec!["text".to_owned(), "audio".to_owned()],
            model: Some("model-a".to_owned()),
        }
    );
    let no_model =
        RealtimeSession::session_event(SessionEventType::Created, None, TRACE_ID, None, &ids);
    assert_eq!(no_model.session.model, None);
    let wire = serde_json::to_string(&created).expect("the created event serializes");
    assert!(wire.contains(r#""event_id":"00000000-0000-4000-8000-000000000011""#));
    assert!(wire.contains(r#""type":"session.created""#));
    assert!(wire.contains(r#""id":"trace-realtime-1""#));
    assert!(wire.contains(r#""modalities":["text","audio"]"#));
    assert!(wire.contains(r#""model":"model-a""#));
}

fn updated_event() {
    let ids = FixedIds::new(&[UPDATED, UPDATED_EMPTY, UPDATED_EXPLICIT, MODEL_EVENT]);
    let updated = RealtimeSession::session_event(
        SessionEventType::Updated,
        Some("model-a"),
        TRACE_ID,
        None,
        &ids,
    );
    assert_eq!(updated.event_type, SessionEventType::Updated);
    assert_eq!(updated.event_id, UPDATED);
    assert_eq!(updated.session.modalities, ["text", "audio"]);
    assert_eq!(updated.session.id, TRACE_ID);
    let no_modalities: &[String] = &[];
    let empty = RealtimeSession::session_event(
        SessionEventType::Updated,
        Some("model-a"),
        TRACE_ID,
        Some(no_modalities),
        &ids,
    );
    assert!(empty.session.modalities.is_empty());
    let explicit = RealtimeSession::session_event(
        SessionEventType::Updated,
        Some("model-a"),
        TRACE_ID,
        Some(&["text".to_owned()]),
        &ids,
    );
    assert_eq!(explicit.session.modalities, ["text"]);
    let model_event =
        RealtimeSession::session_event(SessionEventType::Created, Some(""), TRACE_ID, None, &ids);
    assert_eq!(model_event.event_id, MODEL_EVENT);
    assert_eq!(model_event.session.model, Some(String::new()));
    let wire = serde_json::to_string(&updated).expect("the updated event serializes");
    assert!(wire.contains(r#""type":"session.updated""#));
    assert!(wire.contains(r#""event_id":"00000000-0000-4000-8000-000000000012""#));
}

fn session_event_helpers() {
    const { assert!(REQUIRES_SESSION_CONFIGURATION) };
    let caller = vec![
        ("x-a".to_owned(), "1".to_owned()),
        ("b".to_owned(), "2".to_owned()),
    ];
    assert_eq!(validate_environment(&caller), caller);
    assert!(validate_environment(&[]).is_empty());
    assert_eq!(complete_url(None), "");
    assert_eq!(complete_url(Some("")), "");
    assert_eq!(
        complete_url(Some("https://bedrock.example")),
        "https://bedrock.example"
    );
    let error = error_class(
        "boom".to_owned(),
        429,
        vec![("x-test".to_owned(), "1".to_owned())],
    )
    .expect("the headers are ascii");
    assert_eq!(error.error.status_code, 429);
    assert_eq!(error.error.message, "boom");
    assert_eq!(
        error.error.headers,
        Some(vec![("x-test".to_owned(), "1".to_owned())])
    );
    assert!(!error.error.status_code_is_synthesized);
}

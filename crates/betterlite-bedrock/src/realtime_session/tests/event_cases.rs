use super::support::{CREATED, MODEL_EVENT, TRACE_ID, UPDATED, UPDATED_EMPTY, UPDATED_EXPLICIT};
use crate::invoke_primitives::environment::error_class;
use crate::realtime_session::records::{SessionEventType, SessionObject};
use crate::realtime_session::{REQUIRES_SESSION_CONFIGURATION, RealtimeSession, complete_url};

pub(super) fn created_updated_events() {
    let ids = super::support::FixedIds::new(&[
        CREATED,
        UPDATED,
        UPDATED_EMPTY,
        UPDATED_EXPLICIT,
        MODEL_EVENT,
    ]);
    let created = RealtimeSession::session_created_event(Some("model-a"), TRACE_ID, &ids);
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

    let updated = RealtimeSession::session_updated_event(Some("model-a"), TRACE_ID, None, &ids);
    assert_eq!(updated.event_type, SessionEventType::Updated);
    assert_eq!(updated.event_id, UPDATED);
    assert_eq!(updated.session.modalities, ["text", "audio"]);
    assert_eq!(updated.session.id, TRACE_ID);

    let no_modalities: &[String] = &[];
    let empty = RealtimeSession::session_updated_event(
        Some("model-a"),
        TRACE_ID,
        Some(no_modalities),
        &ids,
    );
    assert!(empty.session.modalities.is_empty());

    let explicit = RealtimeSession::session_updated_event(
        Some("model-a"),
        TRACE_ID,
        Some(&["text".to_owned()]),
        &ids,
    );
    assert_eq!(explicit.session.modalities, ["text"]);

    let no_model = RealtimeSession::session_created_event(None, TRACE_ID, &ids);
    assert_eq!(no_model.session.model, None);
    let empty_model = RealtimeSession::session_created_event(Some(""), TRACE_ID, &ids);
    assert_eq!(empty_model.session.model, Some(String::new()));

    assert!(REQUIRES_SESSION_CONFIGURATION);
    assert_eq!(complete_url(None), "");
    assert_eq!(complete_url(Some("")), "");
    assert_eq!(
        complete_url(Some("https://bedrock.example")),
        "https://bedrock.example"
    );

    let wire = serde_json::to_string(&created).expect("the created event serializes");
    assert!(wire.contains(r#""event_id":"00000000-0000-4000-8000-000000000011""#));
    assert!(wire.contains(r#""type":"session.created""#));
    assert!(wire.contains(r#""id":"trace-realtime-1""#));
    assert!(wire.contains(r#""modalities":["text","audio"]"#));
    assert!(wire.contains(r#""model":"model-a""#));
    let wire = serde_json::to_string(&updated).expect("the updated event serializes");
    assert!(wire.contains(r#""type":"session.updated""#));
    assert!(!wire.contains("event_"));

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

use betterlite_config::api::realtime_session_state::LoggedEventTypes;

use super::support::{self, SECRET};
use crate::realtime_events::close::close_bedrock_client;
use crate::realtime_events::logging::should_log_event;

pub(super) async fn event_logging_selection() {
    default_selection();
    all_selection();
    only_selection();
    close_client().await;
}

fn default_selection() {
    let configured = LoggedEventTypes::Default;
    assert!(should_log_event(
        &configured,
        &support::preface(r#"{"type":"session.created"}"#)
    ));
    assert!(should_log_event(
        &configured,
        &support::preface(r#"{"type":"response.done"}"#)
    ));
    assert!(!should_log_event(
        &configured,
        &support::preface(r#"{"type":"response.created"}"#)
    ));
    assert!(!should_log_event(
        &configured,
        &support::preface(r#"{"type":"session.updated"}"#)
    ));
    assert!(!should_log_event(
        &configured,
        &support::preface(r#"{"type":"conversation.item.input_audio_transcription.completed"}"#)
    ));
    assert!(!should_log_event(&configured, &support::preface("{}")));
    assert!(!should_log_event(
        &configured,
        &support::preface(r#"{"type":3}"#)
    ));
    assert!(!should_log_event(
        &configured,
        &support::preface(r#"{"type":null}"#)
    ));
}

fn all_selection() {
    assert!(should_log_event(
        &LoggedEventTypes::All,
        &support::preface("{}")
    ));
    assert!(should_log_event(
        &LoggedEventTypes::All,
        &support::preface(r#"{"type":"whatever"}"#)
    ));
    assert!(should_log_event(
        &LoggedEventTypes::All,
        &support::preface(r#"{"type":3}"#)
    ));
}

fn only_selection() {
    let configured = LoggedEventTypes::Only(vec!["a".to_owned(), "*".to_owned()]);
    for (type_field, logged) in [
        (r#""type":"a""#, true),
        (r#""type":"*""#, true),
        (r#""type":"b""#, false),
        ("", false),
        (r#""type":3"#, false),
    ] {
        let event = support::preface(&format!("{{{type_field}}}"));
        assert_eq!(
            should_log_event(&configured, &event),
            logged,
            "with {type_field}"
        );
    }
    assert!(!should_log_event(
        &LoggedEventTypes::Only(Vec::new()),
        &support::preface(r#"{"type":"a"}"#)
    ));
}

async fn close_client() {
    let (log, _guard) = support::capture();
    close_bedrock_client(None).await;
    assert!(!log.text().contains("closed SDK client"));
    close_bedrock_client(Some(&support::Closable { error: None })).await;
    assert!(log.text().contains("Bedrock Realtime: closed SDK client"));
    let before = log.text();
    close_bedrock_client(Some(&support::Closable {
        error: Some(SECRET.to_owned()),
    }))
    .await;
    assert_eq!(log.text(), before);
}

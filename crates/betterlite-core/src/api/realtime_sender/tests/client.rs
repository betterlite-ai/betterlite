use super::*;
use crate::api::realtime_sender::error::RealtimeSenderError;

#[tokio::test]
async fn realtime_client_send_translation() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.client_wants_beta.0 = true;
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let done = parsed(r#"{"type":"conversation.item.done","item":{"id":"i"}}"#);
    assert!(!sender.send_event_to_client(&done, "raw").await.unwrap());
    assert_eq!(fixture.client_log.attempts(), 0);
    let delta = parsed(r#"{"type":"response.output_text.delta","delta":"hi"}"#);
    assert!(sender.send_event_to_client(&delta, "raw").await.unwrap());
    assert_eq!(
        fixture.client_log.sent_texts(),
        vec![r#"{"type":"response.text.delta","delta":"hi"}"#.to_owned()]
    );
}

#[tokio::test]
async fn nonobject_events_are_forwarded_untranslated() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.client_wants_beta.0 = true;
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let event = Json::List(vec![Json::Text("x".to_owned())]);
    assert!(
        sender
            .send_event_to_client(&event, "raw-frame")
            .await
            .unwrap()
    );
    assert_eq!(
        fixture.client_log.sent_texts(),
        vec!["raw-frame".to_owned()]
    );
}

#[tokio::test]
async fn normalizer_drops_and_renames_before_sending() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.event_normalizer = Some(Box::new(Normalizer {
        drop_types: vec!["drop.me".to_owned()],
        rename: vec![("old.type".to_owned(), "new.type".to_owned())],
    }));
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let dropped = parsed(r#"{"type":"drop.me"}"#);
    assert!(!sender.send_event_to_client(&dropped, "raw").await.unwrap());
    let renamed = parsed(r#"{"type":"old.type","value":1}"#);
    assert!(sender.send_event_to_client(&renamed, "raw").await.unwrap());
    assert_eq!(
        fixture.client_log.sent_texts(),
        vec![r#"{"type":"new.type","value":1}"#.to_owned()]
    );
    assert_eq!(
        sender
            .event_to_client_json(parsed(r#"{"type":"old.type"}"#))
            .unwrap(),
        r#"{"type":"new.type"}"#
    );
}

#[tokio::test]
async fn beta_translation_failure_falls_back() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.client_wants_beta.0 = true;
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let broken = parsed(r#"{"type":["not","hashable"]}"#);
    assert!(
        sender
            .send_event_to_client(&broken, "unused")
            .await
            .unwrap()
    );
    assert_eq!(fixture.client_log.attempts(), 1);
    assert_eq!(
        fixture.client_log.sent_texts(),
        vec![r#"{"type":["not","hashable"]}"#.to_owned()]
    );
}

#[tokio::test]
async fn beta_send_failure_falls_back_to_the_untranslated_event() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.client_wants_beta.0 = true;
    fixture.client_log.fail_on(0);
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let delta = parsed(r#"{"type":"response.output_text.delta","delta":"hi"}"#);
    assert!(sender.send_event_to_client(&delta, "unused").await.unwrap());
    assert_eq!(fixture.client_log.attempts(), 2);
    assert_eq!(
        fixture.client_log.sent_texts(),
        vec![r#"{"type":"response.output_text.delta","delta":"hi"}"#.to_owned()]
    );
}

#[tokio::test]
async fn failed_fallback_send_is_propagated() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.client_wants_beta.0 = true;
    fixture.client_log.fail_on(0);
    fixture.client_log.fail_on(1);
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let delta = parsed(r#"{"type":"response.output_text.delta","delta":"hi"}"#);
    let result = sender.send_event_to_client(&delta, "unused").await;
    assert!(matches!(result, Err(RealtimeSenderError::Client { .. })));
    assert_eq!(fixture.client_log.attempts(), 2);
}

#[tokio::test]
async fn a_missing_client_fails_the_send() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.client = None;
    let callbacks = no_guardrails();
    let mut sender = make(&mut state, &fixture, &callbacks);
    let event = parsed(r#"{"type":"response.output_text.delta"}"#);
    let result = sender.send_event_to_client(&event, "raw").await;
    assert!(matches!(result, Err(RealtimeSenderError::ClientMissing)));
}

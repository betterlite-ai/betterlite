use super::*;
use crate::api::realtime_sender::error::RealtimeSenderError;

#[tokio::test]
async fn realtime_backend_send_order() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    {
        let mut script = fixture.script();
        script.setup.push(parsed(SETUP_FRAME));
        script.content.push(parsed(CONTENT_FRAME));
        script.transforms.push_back(vec![
            RealtimeMessage::Text(SETUP_FRAME.to_owned()),
            RealtimeMessage::Text(CONTENT_FRAME.to_owned()),
            RealtimeMessage::Binary(BINARY_FRAME.to_vec()),
            RealtimeMessage::Text(SETUP_FRAME.to_owned()),
        ])
    };
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    assert!(sender.send_to_backend("client-frame").await.unwrap());
    assert_eq!(
        fixture.timeline.entries(),
        vec![
            format!("backend:{SETUP_FRAME}"),
            format!("backend:{CONTENT_FRAME}"),
            "pace:binary-frame".to_owned(),
            "backend:binary-frame".to_owned(),
        ]
    );
    assert_eq!(fixture.backend_log.sent().len(), 3);
    assert_eq!(fixture.script().paced, vec![BINARY_FRAME.to_vec()]);
    assert!(state.setup.content_sent_after_setup.0);
    assert_eq!(cached(&state), Some(SETUP_FRAME));
    assert_eq!(fixture.script().seen[0].1, "gpt-realtime");
    assert_eq!(fixture.script().seen[0].2, None);
}

#[tokio::test]
async fn empty_transform_delivers_nothing() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    assert!(!sender.send_to_backend("client-frame").await.unwrap());
    assert_eq!(fixture.backend_log.attempts(), 0);
    assert_eq!(cached(&state), None);
    assert!(!state.setup.content_sent_after_setup.0);
}

#[tokio::test]
async fn failed_setup_send_keeps_the_cache() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.response.session_configuration_request = Some(SECOND_SETUP.to_owned());
    {
        let mut script = fixture.script();
        script.setup.push(parsed(SETUP_FRAME));
        script
            .transforms
            .push_back(vec![RealtimeMessage::Text(SETUP_FRAME.to_owned())])
    };
    fixture.backend_log.fail_on(0);
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let result = sender.send_to_backend("client-frame").await;
    assert!(matches!(result, Err(RealtimeSenderError::Backend { .. })));
    assert_eq!(cached(&state), Some(SECOND_SETUP));
    assert!(!state.setup.content_sent_after_setup.0);
}

#[tokio::test]
async fn failed_content_send_keeps_the_setup_cache() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    {
        let mut script = fixture.script();
        script.setup.push(parsed(SETUP_FRAME));
        script.content.push(parsed(CONTENT_FRAME));
        script.transforms.push_back(vec![
            RealtimeMessage::Text(SETUP_FRAME.to_owned()),
            RealtimeMessage::Text(CONTENT_FRAME.to_owned()),
        ])
    };
    fixture.backend_log.fail_on(1);
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let result = sender.send_to_backend("client-frame").await;
    assert!(matches!(result, Err(RealtimeSenderError::Backend { .. })));
    assert_eq!(cached(&state), Some(SETUP_FRAME));
    assert!(!state.setup.content_sent_after_setup.0);
}

#[tokio::test]
async fn failed_binary_send_leaves_content_unset() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    fixture
        .script()
        .transforms
        .push_back(vec![RealtimeMessage::Binary(BINARY_FRAME.to_vec())]);
    fixture.backend_log.fail_on(0);
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let result = sender.send_to_backend("client-frame").await;
    assert!(matches!(result, Err(RealtimeSenderError::Backend { .. })));
    assert!(!state.setup.content_sent_after_setup.0);
    assert_eq!(fixture.script().paced, vec![BINARY_FRAME.to_vec()]);
}

#[tokio::test]
async fn content_text_marks_content_sent() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    {
        let mut script = fixture.script();
        script.setup.push(parsed(SETUP_FRAME));
        script.content.push(parsed(CONTENT_FRAME));
        script.transforms.push_back(vec![
            RealtimeMessage::Text(CONTENT_FRAME.to_owned()),
            RealtimeMessage::Text(SETUP_FRAME.to_owned()),
        ])
    };
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    assert!(sender.send_to_backend("client-frame").await.unwrap());
    assert!(state.setup.content_sent_after_setup.0);
    assert_eq!(fixture.backend_log.sent().len(), 1);
    assert_eq!(cached(&state), None);
}

#[tokio::test]
async fn malformed_transformed_json_is_sent_as_text() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    fixture
        .script()
        .transforms
        .push_back(vec![RealtimeMessage::Text("{not json".to_owned())]);
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    assert!(sender.send_to_backend("client-frame").await.unwrap());
    assert_eq!(
        fixture.backend_log.sent(),
        vec![RealtimeMessage::Text("{not json".to_owned())]
    );
    assert_eq!(cached(&state), None);
}

#[tokio::test]
async fn direct_backend_send_without_a_provider() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    fixture.script().fail_transform = true;
    let callbacks = no_guardrails();
    let mut sender: RealtimeSender<'_, ScriptedProvider, Infallible> = RealtimeSender {
        state: &mut state,
        provider: None,
        guardrails: &callbacks,
    };
    assert!(sender.send_to_backend("client-frame").await.unwrap());
    assert_eq!(
        fixture.backend_log.sent(),
        vec![RealtimeMessage::Text("client-frame".to_owned())]
    );
    assert!(fixture.script().seen.is_empty());
}

#[tokio::test]
async fn transform_failure_is_returned() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    fixture.script().fail_transform = true;
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let result = sender.send_to_backend("client-frame").await;
    assert!(matches!(result, Err(RealtimeSenderError::Transform { .. })));
    assert_eq!(fixture.backend_log.attempts(), 0);
}

#[tokio::test]
async fn a_missing_backend_fails_the_send() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.backend = None;
    let callbacks = no_guardrails();
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: None::<&ScriptedProvider>,
        guardrails: &callbacks,
    };
    let result = sender.send_to_backend("client-frame").await;
    assert!(matches!(result, Err(RealtimeSenderError::BackendMissing)));
}

#[tokio::test]
async fn enforced_update_reaches_the_backend() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.force_transcription_model = Some("allowed".to_owned());
    let callbacks = no_guardrails();
    let mut sender: RealtimeSender<'_, ScriptedProvider, Infallible> = RealtimeSender {
        state: &mut state,
        provider: None,
        guardrails: &callbacks,
    };
    assert!(sender.send_to_backend(FLAT_UPDATE).await.unwrap());
    let sent = fixture.backend_log.sent();
    assert_eq!(sent.len(), 1);
    let RealtimeMessage::Text(text) = &sent[0] else {
        panic!("the enforced update is a text frame");
    };
    assert_eq!(parsed(text), parsed(FLAT_EXPECTED));
}

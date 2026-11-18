use super::*;
use crate::api::realtime_sender::error::RealtimeSenderError;

fn cache_and_retry_script(fixture: &Fixture) {
    let mut script = fixture.script();
    script
        .setup
        .extend([parsed(SETUP_FRAME), parsed(SECOND_SETUP)]);
    script
        .transforms
        .push_back(vec![RealtimeMessage::Text(SETUP_FRAME.to_owned())]);
    script
        .transforms
        .push_back(vec![RealtimeMessage::Text(SECOND_SETUP.to_owned())]);
    script.transforms.push_back(Vec::new());
    script
        .transforms
        .push_back(vec![RealtimeMessage::Text(GA_DISABLE.to_owned())]);
}

#[tokio::test]
async fn realtime_setup_cache_and_guardrail_retry() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    cache_and_retry_script(&fixture);
    let none = no_guardrails();
    {
        let mut sender = make(&mut state, &fixture, &none);
        assert!(sender.send_to_backend("first").await.unwrap());
    }
    assert_eq!(cached(&state), Some(SETUP_FRAME));
    {
        let mut sender = make(&mut state, &fixture, &none);
        assert!(sender.send_to_backend("second").await.unwrap());
    }
    assert_eq!(cached(&state), Some(SECOND_SETUP));
    let guardrail = AudioGuardrail;
    let callbacks: [&dyn RealtimeGuardrailCallback<Error = Infallible>; 1] = [&guardrail];
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.maybe_disable_auto_response().await.unwrap();
    }
    assert!(!state.setup.turn_detection_update_sent.0);
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.maybe_disable_auto_response().await.unwrap();
    }
    assert!(state.setup.turn_detection_update_sent.0);
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.maybe_disable_auto_response().await.unwrap();
    }
    assert_eq!(fixture.script().seen.len(), 4);
    assert_eq!(fixture.backend_log.sent().len(), 3);
}

#[test]
fn cache_keeps_only_sent_setup_objects() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let callbacks = no_guardrails();
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.cache_session_configuration(r#"{"type":"session.update"}"#);
    }
    assert_eq!(cached(&state), None);
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.cache_session_configuration("not json");
    }
    assert_eq!(cached(&state), None);
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.cache_session_configuration("[1]");
    }
    assert_eq!(cached(&state), None);
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.cache_session_configuration(r#"{"setup":null}"#);
    }
    assert_eq!(cached(&state), Some(r#"{"setup":null}"#));
}

#[tokio::test]
async fn failed_setup_send_keeps_the_injection_flag() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.response.session_configuration_request = Some(SECOND_SETUP.to_owned());
    let guardrail = AudioGuardrail;
    let callbacks: [&dyn RealtimeGuardrailCallback<Error = Infallible>; 1] = [&guardrail];
    {
        let mut script = fixture.script();
        script.setup.push(parsed(SETUP_FRAME));
        script
            .transforms
            .push_back(vec![RealtimeMessage::Text(INJECTED_SETUP.to_owned())]);
    }
    fixture.backend_log.fail_on(0);
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        let result = sender.send_to_backend(SETUP_FRAME).await;
        assert!(matches!(result, Err(RealtimeSenderError::Backend { .. })));
    }
    assert!(state.setup.turn_detection_update_sent.0);
    assert_eq!(cached(&state), Some(SECOND_SETUP));
    {
        let mut sender = make(&mut state, &fixture, &callbacks);
        sender.maybe_disable_auto_response().await.unwrap();
    }
    assert_eq!(fixture.script().seen.len(), 1);
}

#[tokio::test]
async fn ga_disable_update_nests_turn_detection() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let guardrail = AudioGuardrail;
    let callbacks: [&dyn RealtimeGuardrailCallback<Error = Infallible>; 1] = [&guardrail];
    fixture
        .script()
        .transforms
        .push_back(vec![RealtimeMessage::Text(GA_DISABLE.to_owned())]);
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    sender.maybe_disable_auto_response().await.unwrap();
    assert!(state.setup.turn_detection_update_sent.0);
    assert_eq!(parsed(&fixture.script().seen[0].0), parsed(GA_DISABLE));
}

#[tokio::test]
async fn beta_disable_update_is_a_flat_turn_detection() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.backend_uses_beta_protocol.0 = true;
    let guardrail = AudioGuardrail;
    let callbacks: [&dyn RealtimeGuardrailCallback<Error = Infallible>; 1] = [&guardrail];
    fixture
        .script()
        .transforms
        .push_back(vec![RealtimeMessage::Text(BETA_DISABLE.to_owned())]);
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    sender.maybe_disable_auto_response().await.unwrap();
    assert_eq!(parsed(&fixture.script().seen[0].0), parsed(BETA_DISABLE));
}

#[tokio::test]
async fn other_hooks_send_no_disable_update() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let guardrail = PreCallGuardrail;
    let callbacks: [&dyn RealtimeGuardrailCallback<Error = Infallible>; 1] = [&guardrail];
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    sender.maybe_disable_auto_response().await.unwrap();
    assert!(!state.setup.turn_detection_update_sent.0);
    assert!(fixture.script().seen.is_empty());
    assert_eq!(fixture.backend_log.attempts(), 0);
}

#[tokio::test]
async fn a_failing_guardrail_predicate_is_returned() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let guardrail = FailingGuardrail;
    let callbacks: [&dyn RealtimeGuardrailCallback<Error = GuardFailure>; 1] = [&guardrail];
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let result = sender.maybe_disable_auto_response().await;
    assert!(matches!(result, Err(RealtimeSenderError::Predicate { .. })));
}

#[tokio::test]
async fn a_failed_guardrail_patch_is_returned() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let guardrail = AudioGuardrail;
    let callbacks: [&dyn RealtimeGuardrailCallback<Error = Infallible>; 1] = [&guardrail];
    let broken = r#"{"setup":{"realtimeInputConfig":"flat"}}"#;
    {
        let mut script = fixture.script();
        script.setup.push(parsed(broken));
        script
            .transforms
            .push_back(vec![RealtimeMessage::Text(broken.to_owned())]);
    }
    let mut sender = RealtimeSender {
        state: &mut state,
        provider: Some(&fixture.provider),
        guardrails: &callbacks,
    };
    let result = sender.send_to_backend(broken).await;
    assert!(matches!(
        result,
        Err(RealtimeSenderError::GuardrailSetup { .. })
    ));
    assert_eq!(fixture.backend_log.attempts(), 0);
}

#![cfg(test)]

use std::future::Future;
use std::thread;

use tokio::runtime::Builder;

use super::*;

// Moves the work to a second thread, so its future has to be Send and carries the sender with it.
fn on_second_thread<Output, Work>(work: Work) -> Output
where
    Output: Send,
    Work: Future<Output = Output> + Send,
{
    thread::scope(|scope| {
        scope
            .spawn(|| {
                Builder::new_current_thread()
                    .build()
                    .unwrap()
                    .block_on(work)
            })
            .join()
            .unwrap()
    })
}

#[test]
fn send_to_backend_completes_on_a_second_thread() {
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
        ]);
    };
    let callbacks = no_guardrails();
    let (sent, timeline, cache) = on_second_thread(async move {
        let sent = {
            let mut sender = make(&mut state, &fixture, &callbacks);
            sender.send_to_backend("client-frame").await.unwrap()
        };
        let cache = cached(&state).map(str::to_owned);
        (sent, fixture.timeline.entries(), cache)
    });
    assert!(sent);
    assert_eq!(
        timeline,
        vec![
            format!("backend:{SETUP_FRAME}"),
            format!("backend:{CONTENT_FRAME}"),
            "pace:binary-frame".to_owned(),
            "backend:binary-frame".to_owned(),
        ]
    );
    assert_eq!(cache.as_deref(), Some(SETUP_FRAME));
}

#[test]
fn send_event_to_client_completes_on_a_second_thread() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    state.event_normalizer = Some(Box::new(Normalizer {
        drop_types: vec!["drop.me".to_owned()],
        rename: vec![("old.type".to_owned(), "new.type".to_owned())],
    }));
    let callbacks = no_guardrails();
    let (dropped, renamed, sent) = on_second_thread(async move {
        let mut sender = make(&mut state, &fixture, &callbacks);
        let dropped_event = parsed(r#"{"type":"drop.me"}"#);
        let dropped = sender
            .send_event_to_client(&dropped_event, "raw")
            .await
            .unwrap();
        let renamed_event = parsed(r#"{"type":"old.type","value":1}"#);
        let renamed = sender
            .send_event_to_client(&renamed_event, "raw")
            .await
            .unwrap();
        (dropped, renamed, fixture.client_log.sent())
    });
    assert!(!dropped);
    assert!(renamed);
    assert_eq!(sent, vec![r#"{"type":"new.type","value":1}"#.to_owned()]);
}

#[test]
fn maybe_disable_auto_response_completes_on_a_second_thread() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    fixture
        .script()
        .transforms
        .push_back(vec![RealtimeMessage::Text(GA_DISABLE.to_owned())]);
    let (disabled, seen) = on_second_thread(async move {
        let guardrail = AudioGuardrail;
        let callbacks: [&dyn RealtimeGuardrailCallback<Error = Infallible>; 1] = [&guardrail];
        {
            let mut sender = make(&mut state, &fixture, &callbacks);
            sender.maybe_disable_auto_response().await.unwrap();
        };
        let disabled = state.setup.turn_detection_update_sent.0;
        let seen = fixture.script().seen[0].0.clone();
        (disabled, seen)
    });
    assert!(disabled);
    assert_eq!(parsed(&seen), parsed(GA_DISABLE));
}

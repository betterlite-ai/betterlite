use betterlite_models::decisions::Json;

use crate::api::realtime_contracts::close::BackendClose;

use super::harness::*;

fn backend_close(code: u16, reason: &str) -> BackendClose {
    BackendClose {
        code,
        reason: reason.to_owned(),
    }
}

fn event_message(outcome: &Outcome) -> String {
    let frames = outcome.client_frames();
    assert_eq!(frames.len(), 1);
    let Json::Object(root) = &frames[0] else {
        panic!("the close notice is an event object");
    };
    let Some(Json::Object(error)) = root.get("error") else {
        panic!("the close notice carries an error object");
    };
    match error.get("message") {
        Some(Json::Text(message)) => message.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => panic!("the close notice carries an error message"),
    }
}

#[tokio::test]
async fn realtime_client_close_redaction() {
    normal_closure_sends_no_event().await;
    an_error_closure_emits_a_server_error().await;
    an_abnormal_upstream_code_maps_to_1011().await;
    an_empty_reason_falls_back_to_the_close_message().await;
    a_long_reason_is_cut_on_a_character_boundary().await;
    a_secret_reason_is_redacted_in_both_places().await;
    a_failed_notice_skips_the_close().await;
    a_failed_close_is_logged_and_swallowed().await;
}

async fn normal_closure_sends_no_event() {
    let outcome = close(Fixture::default(), backend_close(1000, "normal")).await;
    assert!(outcome.client_texts().is_empty());
    assert_eq!(outcome.closes(), vec![(1000, Some(String::from("normal")))]);
}

async fn an_error_closure_emits_a_server_error() {
    let outcome = close(Fixture::default(), backend_close(1008, "policy")).await;
    assert_eq!(
        event_message(&outcome),
        "upstream websocket closed with code 1008: policy"
    );
    assert_eq!(outcome.closes(), vec![(1008, Some(String::from("policy")))]);
}

async fn an_abnormal_upstream_code_maps_to_1011() {
    let outcome = close(Fixture::default(), backend_close(1006, "")).await;
    assert_eq!(
        event_message(&outcome),
        "upstream websocket closed with code 1006"
    );
    assert_eq!(
        outcome.closes(),
        vec![(
            1011,
            Some(String::from("upstream websocket closed with code 1006"))
        )]
    );
}

async fn an_empty_reason_falls_back_to_the_close_message() {
    let outcome = close(Fixture::default(), backend_close(1008, "")).await;
    assert_eq!(
        outcome.closes(),
        vec![(
            1008,
            Some(String::from("upstream websocket closed with code 1008"))
        )]
    );
}

async fn a_long_reason_is_cut_on_a_character_boundary() {
    let reason = "\u{e9}".repeat(100);
    let outcome = close(Fixture::default(), backend_close(1008, &reason)).await;
    let closes = outcome.closes();
    assert_eq!(closes.len(), 1);
    let Some((_, Some(wire_reason))) = closes.first() else {
        panic!("the client close carries a reason");
    };
    assert!(wire_reason.len() <= 123);
    assert_eq!(wire_reason, &"\u{e9}".repeat(61));
}

async fn a_secret_reason_is_redacted_in_both_places() {
    let outcome = close(
        Fixture::default().with_redactor(),
        backend_close(1008, "sk-abcdefghijklmno leaked"),
    )
    .await;
    let message = event_message(&outcome);
    assert!(!message.contains("sk-abcdefghijklmno"));
    assert!(message.contains("REDACTED"));
    let closes = outcome.closes();
    assert_eq!(closes, vec![(1008, Some(String::from("REDACTED leaked")))]);
}

async fn a_failed_notice_skips_the_close() {
    let (logs, _guard) = capture();
    let outcome = close(
        Fixture::default().with_client_send_failures(vec![0]),
        backend_close(1008, "policy"),
    )
    .await;
    assert!(outcome.closes().is_empty());
    assert!(logs.starts_with("Could not relay the upstream close to the client: "));
}

async fn a_failed_close_is_logged_and_swallowed() {
    let (logs, _guard) = capture();
    let outcome = close(
        Fixture::default().with_client_close_failures(vec![0]),
        backend_close(1008, "policy"),
    )
    .await;
    assert_eq!(outcome.client_texts().len(), 1);
    assert_eq!(outcome.closes(), vec![(1008, Some(String::from("policy")))]);
    assert!(logs.starts_with("Could not relay the upstream close to the client: "));
}

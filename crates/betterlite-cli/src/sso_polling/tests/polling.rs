use std::time::Duration;

use crate::sso_polling::polling::PollNotice;

use super::fixtures::{
    Echo, RecordingSleeper, Reply, pending, poll, repeated, response, scripted_transport,
    wait_options,
};

#[test]
fn polling_logs_the_pending_message_at_attempt_zero_and_every_interval() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(
        repeated(&pending(), 11)
            .into_iter()
            .chain([Reply::Response(response(200, r#"{"status":"ready"}"#))])
            .collect(),
    );

    let data = poll(&transport, &wait_options(), &sleeper, &echo).expect("the poll");

    assert!(data.is_some());
    assert_eq!(echo.recorded(), vec!["pending message", "pending message"]);
    assert_eq!(sleeper.intervals(), vec![Duration::from_secs(2); 11]);
    assert_eq!(transport.calls().len(), 12);
}

#[test]
fn polling_logs_the_other_status_message_for_an_unknown_status() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![
        Reply::Response(response(200, r#"{"status":"processing"}"#)),
        Reply::Response(response(200, r#"{"status":"ready"}"#)),
    ]);

    let data = poll(&transport, &wait_options(), &sleeper, &echo).expect("the poll");

    assert!(data.is_some());
    assert_eq!(echo.recorded(), vec!["other status message"]);
    assert_eq!(sleeper.intervals(), vec![Duration::from_secs(2)]);
}

#[test]
fn polling_logs_repeated_http_errors_at_the_configured_interval() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let failure = Reply::Response(response(500, r#"{"detail":"down"}"#));
    let transport = scripted_transport(
        repeated(&failure, 11)
            .into_iter()
            .chain([Reply::Response(response(200, r#"{"status":"ready"}"#))])
            .collect(),
    );

    let data = poll(&transport, &wait_options(), &sleeper, &echo).expect("the poll");

    assert!(data.is_some());
    assert_eq!(
        echo.recorded(),
        vec![
            "Polling error: HTTP 500: down",
            "Polling error: HTTP 500: down"
        ]
    );
    assert_eq!(transport.calls().len(), 12);
}

#[test]
fn polling_keeps_polling_after_a_connection_error() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![
        Reply::Transport,
        Reply::Response(response(200, r#"{"status":"ready"}"#)),
    ]);

    let data = poll(&transport, &wait_options(), &sleeper, &echo).expect("the poll");

    assert!(data.is_some());
    assert_eq!(
        echo.recorded(),
        vec!["Connection error (will retry): connection refused"]
    );
    assert_eq!(sleeper.intervals(), vec![Duration::from_secs(2)]);
}

#[test]
fn polling_stays_quiet_when_the_pending_log_interval_is_zero() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let mut options = wait_options();
    options.pending = Some(PollNotice {
        message: String::from("pending message"),
        every: 0,
    });
    let transport = scripted_transport(vec![
        pending(),
        Reply::Response(response(200, r#"{"status":"ready"}"#)),
    ]);

    let data = poll(&transport, &options, &sleeper, &echo).expect("the poll");

    assert!(data.is_some());
    assert!(echo.recorded().is_empty());
}

#[test]
fn polling_stays_quiet_without_a_pending_message() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let mut options = wait_options();
    options.pending = None;
    let transport = scripted_transport(vec![
        pending(),
        Reply::Response(response(200, r#"{"status":"ready"}"#)),
    ]);

    let data = poll(&transport, &options, &sleeper, &echo).expect("the poll");

    assert!(data.is_some());
    assert!(echo.recorded().is_empty());
}

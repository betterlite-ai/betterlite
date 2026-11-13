use std::time::Duration;

use crate::sso_polling::polling::PollNotice;

use super::fixtures::{
    Echo, PollFixture, Reply, pending, repeated, response, scripted_transport, wait_options,
};

#[test]
fn polling_logs_the_pending_message_at_attempt_zero_and_every_interval() {
    let fixture = PollFixture::new(
        scripted_transport(
            repeated(&pending(), 11)
                .into_iter()
                .chain([Reply::Response(response(200, r#"{"status":"ready"}"#))])
                .collect(),
        ),
        Echo::default(),
    );

    let data = fixture.poll(&wait_options()).expect("the poll");

    assert!(data.is_some());
    assert_eq!(
        fixture.echo.recorded(),
        vec!["pending message", "pending message"]
    );
    assert_eq!(
        fixture.sleeper.intervals(),
        vec![Duration::from_secs(2); 11]
    );
    assert_eq!(fixture.transport.calls().len(), 12);
}

#[test]
fn polling_logs_the_other_status_message_for_an_unknown_status() {
    let fixture = PollFixture::new(
        scripted_transport(vec![
            Reply::Response(response(200, r#"{"status":"processing"}"#)),
            Reply::Response(response(200, r#"{"status":"ready"}"#)),
        ]),
        Echo::default(),
    );

    let data = fixture.poll(&wait_options()).expect("the poll");

    assert!(data.is_some());
    assert_eq!(fixture.echo.recorded(), vec!["other status message"]);
    assert_eq!(fixture.sleeper.intervals(), vec![Duration::from_secs(2)]);
}

#[test]
fn polling_logs_repeated_http_errors_at_the_configured_interval() {
    let failure = Reply::Response(response(500, r#"{"detail":"down"}"#));
    let fixture = PollFixture::new(
        scripted_transport(
            repeated(&failure, 11)
                .into_iter()
                .chain([Reply::Response(response(200, r#"{"status":"ready"}"#))])
                .collect(),
        ),
        Echo::default(),
    );

    let data = fixture.poll(&wait_options()).expect("the poll");

    assert!(data.is_some());
    assert_eq!(
        fixture.echo.recorded(),
        vec![
            "Polling error: HTTP 500: down",
            "Polling error: HTTP 500: down"
        ]
    );
    assert_eq!(fixture.transport.calls().len(), 12);
}

#[test]
fn polling_keeps_polling_after_a_connection_error() {
    let fixture = PollFixture::new(
        scripted_transport(vec![
            Reply::Transport,
            Reply::Response(response(200, r#"{"status":"ready"}"#)),
        ]),
        Echo::default(),
    );

    let data = fixture.poll(&wait_options()).expect("the poll");

    assert!(data.is_some());
    assert_eq!(
        fixture.echo.recorded(),
        vec!["Connection error (will retry): connection refused"]
    );
    assert_eq!(fixture.sleeper.intervals(), vec![Duration::from_secs(2)]);
}

#[test]
fn polling_stays_quiet_when_the_pending_log_interval_is_zero() {
    let mut options = wait_options();
    options.pending = Some(PollNotice {
        message: String::from("pending message"),
        every: 0,
    });
    let fixture = PollFixture::new(
        scripted_transport(vec![
            pending(),
            Reply::Response(response(200, r#"{"status":"ready"}"#)),
        ]),
        Echo::default(),
    );

    let data = fixture.poll(&options).expect("the poll");

    assert!(data.is_some());
    assert!(fixture.echo.recorded().is_empty());
}

#[test]
fn polling_stays_quiet_without_a_pending_message() {
    let mut options = wait_options();
    options.pending = None;
    let fixture = PollFixture::new(
        scripted_transport(vec![
            pending(),
            Reply::Response(response(200, r#"{"status":"ready"}"#)),
        ]),
        Echo::default(),
    );

    let data = fixture.poll(&options).expect("the poll");

    assert!(data.is_some());
    assert!(fixture.echo.recorded().is_empty());
}

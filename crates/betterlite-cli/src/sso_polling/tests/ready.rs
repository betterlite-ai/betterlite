use std::time::Duration;

use secrecy::ExposeSecret as _;

use super::fixtures::{
    Echo, POLL_URL, RecordingSleeper, Reply, pending, poll, repeated, response, scripted_transport,
    wait_options,
};

#[test]
fn ready_returns_the_poll_data_when_the_status_is_ready() {
    let body = r#"{"status":"ready","key":"sk-key","user_id":"user-1","teams":["team-1"],"team_details":[{"team_id":"team-1"}],"requires_team_selection":false,"team_id":"team-1"}"#;
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![Reply::Response(response(200, body))]);

    let data = poll(&transport, &wait_options(), &sleeper, &echo)
        .expect("the poll")
        .expect("the ready data");

    assert_eq!(data.status.as_deref(), Some("ready"));
    assert_eq!(
        data.key.as_ref().expect("the key").expose_secret(),
        "sk-key"
    );
    assert_eq!(data.user_id.as_deref(), Some("user-1"));
    assert_eq!(data.teams, vec!["team-1"]);
    assert_eq!(
        data.team_details.as_ref().expect("the details").get(),
        r#"[{"team_id":"team-1"}]"#
    );
    assert!(!data.requires_team_selection);
    assert_eq!(data.team_id.as_deref(), Some("team-1"));
    assert!(sleeper.intervals().is_empty());
    assert!(echo.recorded().is_empty());
}

#[test]
fn ready_returns_none_when_the_timeout_budget_runs_out() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let mut options = wait_options();
    options.total_timeout_seconds = 5;
    let transport = scripted_transport(repeated(&pending(), 2));

    let data = poll(&transport, &options, &sleeper, &echo).expect("the poll");

    assert!(data.is_none());
    assert_eq!(transport.calls().len(), 2);
    assert_eq!(sleeper.intervals(), vec![Duration::from_secs(2); 2]);
}

#[test]
fn ready_rejects_a_permanent_status_with_its_detail() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![Reply::Response(response(
        403,
        r#"{"detail":"denied"}"#,
    ))]);

    let error = poll(&transport, &wait_options(), &sleeper, &echo).expect_err("the rejection");

    assert_eq!(
        error.to_string(),
        "the proxy rejected the login session with HTTP 403: denied"
    );
    assert!(sleeper.intervals().is_empty());
}

#[test]
fn ready_rejects_a_permanent_status_without_a_detail() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![Reply::Response(response(400, ""))]);

    let error = poll(&transport, &wait_options(), &sleeper, &echo).expect_err("the rejection");

    assert_eq!(
        error.to_string(),
        format!(
            "the proxy rejected the login session with HTTP 400 and no error detail (from {POLL_URL})"
        )
    );
}

#[test]
fn ready_keeps_polling_through_a_rate_limited_status() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![
        Reply::Response(response(429, "")),
        Reply::Response(response(200, r#"{"status":"ready"}"#)),
    ]);

    let data = poll(&transport, &wait_options(), &sleeper, &echo).expect("the poll");

    assert!(data.is_some());
    assert_eq!(echo.recorded(), vec!["Polling error: HTTP 429"]);
}

#[test]
fn ready_fails_when_a_success_body_is_not_poll_data() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![Reply::Response(response(200, "not json"))]);

    let error = poll(&transport, &wait_options(), &sleeper, &echo).expect_err("the body");

    assert!(
        error
            .to_string()
            .contains("does not match the poll data shape")
    );
}

#[test]
fn ready_names_a_wrongly_typed_poll_field() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let transport = scripted_transport(vec![Reply::Response(response(200, r#"{"status":5}"#))]);

    let error = poll(&transport, &wait_options(), &sleeper, &echo).expect_err("the body");

    assert!(error.to_string().contains("status"));
}

#[test]
fn ready_rejects_a_zero_poll_interval() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let mut options = wait_options();
    options.poll_interval_seconds = 0;
    let transport = scripted_transport(Vec::new());

    let error = poll(&transport, &options, &sleeper, &echo).expect_err("the interval");

    assert_eq!(
        error.to_string(),
        "the poll interval must be greater than zero"
    );
    assert!(transport.calls().is_empty());
}

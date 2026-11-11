use std::time::Duration;

use secrecy::ExposeSecret as _;

use super::fixtures::{
    Echo, POLL_URL, PollFixture, Reply, pending, repeated, response, scripted_transport,
    wait_options,
};

#[test]
fn ready_returns_the_poll_data_when_the_status_is_ready() {
    let body = r#"{"status":"ready","key":"sk-key","user_id":"user-1","teams":["team-1"],"team_details":[{"team_id":"team-1"}],"requires_team_selection":false,"team_id":"team-1"}"#;
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(200, body))]),
        Echo::default(),
    );

    let data = fixture
        .poll(&wait_options())
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
    assert!(fixture.sleeper.intervals().is_empty());
    assert!(fixture.echo.recorded().is_empty());
}

#[test]
fn ready_returns_none_when_the_timeout_budget_runs_out() {
    let mut options = wait_options();
    options.total_timeout_seconds = 5;
    let fixture = PollFixture::new(scripted_transport(repeated(&pending(), 2)), Echo::default());

    let data = fixture.poll(&options).expect("the poll");

    assert!(data.is_none());
    assert_eq!(fixture.transport.calls().len(), 2);
    assert_eq!(fixture.sleeper.intervals(), vec![Duration::from_secs(2); 2]);
}

#[test]
fn ready_rejects_a_permanent_status_with_its_detail() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(
            403,
            r#"{"detail":"denied"}"#,
        ))]),
        Echo::default(),
    );

    let error = fixture.poll(&wait_options()).expect_err("the rejection");

    assert_eq!(
        error.to_string(),
        "the proxy rejected the login session with HTTP 403: denied"
    );
    assert!(fixture.sleeper.intervals().is_empty());
}

#[test]
fn ready_rejects_a_permanent_status_without_a_detail() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(400, ""))]),
        Echo::default(),
    );

    let error = fixture.poll(&wait_options()).expect_err("the rejection");

    assert_eq!(
        error.to_string(),
        format!(
            "the proxy rejected the login session with HTTP 400 and no error detail (from {POLL_URL})"
        )
    );
}

#[test]
fn ready_keeps_polling_through_a_rate_limited_status() {
    let fixture = PollFixture::new(
        scripted_transport(vec![
            Reply::Response(response(429, "")),
            Reply::Response(response(200, r#"{"status":"ready"}"#)),
        ]),
        Echo::default(),
    );

    let data = fixture.poll(&wait_options()).expect("the poll");

    assert!(data.is_some());
    assert_eq!(fixture.echo.recorded(), vec!["Polling error: HTTP 429"]);
}

#[test]
fn ready_retries_when_a_success_body_is_not_json() {
    let fixture = PollFixture::new(
        scripted_transport(vec![
            Reply::Response(response(200, "not json")),
            Reply::Response(response(200, r#"{"status":"ready"}"#)),
        ]),
        Echo::default(),
    );

    let data = fixture.poll(&wait_options()).expect("the poll");

    assert!(data.is_some());
    let notice = fixture.echo.recorded();
    assert_eq!(notice.len(), 1);
    assert!(notice[0].starts_with("Connection error (will retry): "));
    assert_eq!(fixture.sleeper.intervals(), vec![Duration::from_secs(2)]);
    assert_eq!(fixture.transport.calls().len(), 2);
}

#[test]
fn ready_notices_a_non_json_success_body_on_the_connection_error_schedule() {
    let mut options = wait_options();
    options.http_error_log_every = 3;
    options.connection_error_log_every = 5;
    let broken = Reply::Response(response(200, "not json"));
    let fixture = PollFixture::new(
        scripted_transport(
            repeated(&broken, 10)
                .into_iter()
                .chain([Reply::Response(response(200, r#"{"status":"ready"}"#))])
                .collect(),
        ),
        Echo::default(),
    );

    let data = fixture.poll(&options).expect("the poll");

    assert!(data.is_some());
    let notices = fixture.echo.recorded();
    assert_eq!(notices.len(), 2);
    assert!(
        notices
            .iter()
            .all(|message| message.starts_with("Connection error (will retry): "))
    );
}

#[test]
fn ready_retries_when_json_is_followed_by_trailing_bytes() {
    let fixture = PollFixture::new(
        scripted_transport(vec![
            Reply::Response(response(200, r#"{"status":"ready"} trailing"#)),
            Reply::Response(response(200, r#"{"status":"ready"}"#)),
        ]),
        Echo::default(),
    );

    let data = fixture.poll(&wait_options()).expect("the poll");

    assert!(data.is_some());
    assert_eq!(fixture.echo.recorded().len(), 1);
    let notice = fixture.echo.recorded();
    assert!(notice[0].starts_with("Connection error (will retry): "));
    assert_eq!(fixture.sleeper.intervals(), vec![Duration::from_secs(2)]);
}

#[test]
fn ready_times_out_when_the_success_body_is_never_json() {
    let mut options = wait_options();
    options.total_timeout_seconds = 10;
    let broken = Reply::Response(response(200, "<html>bad gateway</html>"));
    let fixture = PollFixture::new(scripted_transport(repeated(&broken, 5)), Echo::default());

    let data = fixture.poll(&options).expect("the poll");

    assert!(data.is_none());
    assert_eq!(fixture.transport.calls().len(), 5);
    assert_eq!(fixture.sleeper.intervals(), vec![Duration::from_secs(2); 5]);
    let notice = fixture.echo.recorded();
    assert_eq!(notice.len(), 1);
    assert!(notice[0].starts_with("Connection error (will retry): "));
}

#[test]
fn ready_rejects_a_success_body_that_is_not_an_object() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(200, r#"["ready"]"#))]),
        Echo::default(),
    );

    let error = fixture.poll(&wait_options()).expect_err("the array body");

    assert!(
        error
            .to_string()
            .contains("does not match the poll data shape")
    );
}

#[test]
fn ready_names_a_wrongly_typed_poll_field() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(200, r#"{"status":5}"#))]),
        Echo::default(),
    );

    let error = fixture.poll(&wait_options()).expect_err("the body");

    assert!(error.to_string().contains("status"));
}

#[test]
fn ready_rejects_a_zero_poll_interval() {
    let mut options = wait_options();
    options.poll_interval_seconds = 0;
    let fixture = PollFixture::new(scripted_transport(Vec::new()), Echo::default());

    let error = fixture.poll(&options).expect_err("the interval");

    assert_eq!(
        error.to_string(),
        "the poll interval must be greater than zero"
    );
    assert!(fixture.transport.calls().is_empty());
}

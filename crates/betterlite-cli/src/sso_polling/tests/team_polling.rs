use secrecy::ExposeSecret as _;

use crate::credentials::contracts::CliTeam;

use super::fixtures::{
    Echo, POLL_SECRET, PollCall, PollFixture, Reply, SERVER, pending, piped_input, repeated,
    response, scripted_transport, team,
};

fn teams() -> Vec<CliTeam> {
    vec![team("team-1", Some("Alpha")), team("team-2", Some("Beta"))]
}

#[test]
fn team_polling_repolls_with_the_chosen_team_and_returns_the_jwt() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(
            200,
            r#"{"status":"ready","key":"jwt-team-2"}"#,
        ))]),
        Echo::default(),
    );
    let mut input = piped_input(vec!["2"]);

    let jwt = fixture
        .select_team(&mut input, &teams())
        .expect("the team poll")
        .expect("the jwt");

    assert_eq!(jwt.expose_secret(), "jwt-team-2");
    assert_eq!(
        fixture.transport.calls(),
        vec![PollCall::Get {
            url: format!("{SERVER}/sso/cli/poll/login-1?team_id=team-2"),
            headers: vec![(
                String::from("x-litellm-cli-poll-secret"),
                String::from(POLL_SECRET)
            )],
            timeout: 10,
        }]
    );
    let messages = fixture.echo.recorded();
    assert_eq!(messages[1], "Select a team for your CLI session...");
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Generating JWT for team: team-2"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Successfully generated JWT for team: team-2"))
    );
}

#[test]
fn team_polling_uses_the_first_team_when_the_user_skips() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(
            200,
            r#"{"status":"ready","key":"jwt-team-1"}"#,
        ))]),
        Echo::default(),
    );
    let mut input = piped_input(vec!["skip"]);

    let jwt = fixture
        .select_team(&mut input, &teams())
        .expect("the team poll")
        .expect("the jwt");

    assert_eq!(jwt.expose_secret(), "jwt-team-1");
    assert_eq!(
        fixture.transport.calls()[0],
        PollCall::Get {
            url: format!("{SERVER}/sso/cli/poll/login-1?team_id=team-1"),
            headers: vec![(
                String::from("x-litellm-cli-poll-secret"),
                String::from(POLL_SECRET)
            )],
            timeout: 10,
        }
    );
}

#[test]
fn team_polling_returns_none_without_teams() {
    let fixture = PollFixture::new(scripted_transport(Vec::new()), Echo::default());
    let mut input = piped_input(Vec::new());

    let jwt = fixture.select_team(&mut input, &[]).expect("the team poll");

    assert!(jwt.is_none());
    assert!(fixture.transport.calls().is_empty());
    assert!(
        fixture
            .echo
            .recorded()
            .iter()
            .any(|message| message.contains("No teams found"))
    );
}

#[test]
fn team_polling_returns_none_when_the_repoll_has_no_key() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(
            200,
            r#"{"status":"ready"}"#,
        ))]),
        Echo::default(),
    );
    let mut input = piped_input(vec!["1"]);

    let jwt = fixture
        .select_team(&mut input, &teams())
        .expect("the team poll");

    assert!(jwt.is_none());
    assert!(
        fixture
            .echo
            .recorded()
            .iter()
            .all(|message| !message.contains("Successfully generated JWT"))
    );
}

#[test]
fn team_polling_returns_none_when_the_repoll_times_out() {
    let fixture = PollFixture::new(
        scripted_transport(repeated(&pending(), 150)),
        Echo::default(),
    );
    let mut input = piped_input(vec!["1"]);

    let jwt = fixture
        .select_team(&mut input, &teams())
        .expect("the team poll");

    assert!(jwt.is_none());
    assert_eq!(fixture.transport.calls().len(), 150);
}

#[test]
fn team_polling_reports_a_permanent_rejection() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(
            403,
            r#"{"detail":"not allowed"}"#,
        ))]),
        Echo::default(),
    );
    let mut input = piped_input(vec!["1"]);

    let error = fixture
        .select_team(&mut input, &teams())
        .expect_err("the rejection");

    assert!(error.to_string().contains("not allowed"));
}

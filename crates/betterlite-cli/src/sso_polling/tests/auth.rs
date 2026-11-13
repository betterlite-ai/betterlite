use secrecy::ExposeSecret as _;

use super::fixtures::{
    Echo, POLL_SECRET, PollCall, PollFixture, Reply, SERVER, pending, piped_input, repeated,
    response, scripted_transport,
};

const AUTHENTICATED: &str =
    r#"{"status":"ready","key":"sk-key","user_id":"user-1","teams":["team-1"],"team_id":"team-1"}"#;
const TEAM_CHOICE: &str = r#"{"status":"ready","requires_team_selection":true,"user_id":"user-1","teams":["team-1","team-2"],"team_details":[{"team_id":"team-1","team_alias":"Alpha"},{"id":"team-2"}]}"#;

#[test]
fn auth_returns_the_key_identity_and_teams() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(200, AUTHENTICATED))]),
        Echo::default(),
    );
    let mut input = piped_input(Vec::new());

    let result = fixture
        .authenticate(&mut input)
        .expect("the auth")
        .expect("the result");

    assert_eq!(result.api_key.expose_secret(), "sk-key");
    assert_eq!(result.user_id.as_deref(), Some("user-1"));
    assert_eq!(result.teams, vec!["team-1"]);
    assert_eq!(result.team_id.as_deref(), Some("team-1"));
    assert!(
        fixture
            .echo
            .recorded()
            .iter()
            .any(|message| message.contains("Automatically assigned to team: team-1"))
    );
    assert_eq!(
        fixture.transport.calls(),
        vec![PollCall::Get {
            url: format!("{SERVER}/sso/cli/poll/login-1"),
            headers: vec![(
                String::from("x-litellm-cli-poll-secret"),
                String::from(POLL_SECRET)
            )],
            timeout: 10,
        }]
    );
}

#[test]
fn auth_does_not_announce_a_team_when_several_are_listed() {
    let body =
        r#"{"status":"ready","key":"sk-key","teams":["team-1","team-2"],"team_id":"team-1"}"#;
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(200, body))]),
        Echo::default(),
    );
    let mut input = piped_input(Vec::new());

    let result = fixture
        .authenticate(&mut input)
        .expect("the auth")
        .expect("the result");

    assert_eq!(result.teams, vec!["team-1", "team-2"]);
    assert!(
        fixture
            .echo
            .recorded()
            .iter()
            .all(|message| !message.contains("Automatically assigned"))
    );
}

#[test]
fn auth_returns_none_when_no_key_is_ready() {
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(
            200,
            r#"{"status":"ready","user_id":"user-1"}"#,
        ))]),
        Echo::default(),
    );
    let mut input = piped_input(Vec::new());

    let result = fixture.authenticate(&mut input).expect("the auth");

    assert!(result.is_none());
}

#[test]
fn auth_returns_none_when_the_poll_times_out() {
    let fixture = PollFixture::new(
        scripted_transport(repeated(&pending(), 150)),
        Echo::default(),
    );
    let mut input = piped_input(Vec::new());

    let result = fixture.authenticate(&mut input).expect("the auth");

    assert!(result.is_none());
    assert_eq!(fixture.transport.calls().len(), 150);
}

#[test]
fn auth_returns_none_when_no_teams_can_be_selected() {
    let body = r#"{"status":"ready","requires_team_selection":true,"user_id":"user-1","teams":[]}"#;
    let fixture = PollFixture::new(
        scripted_transport(vec![Reply::Response(response(200, body))]),
        Echo::default(),
    );
    let mut input = piped_input(Vec::new());

    let result = fixture.authenticate(&mut input).expect("the auth");

    assert!(result.is_none());
    assert!(
        fixture
            .echo
            .recorded()
            .iter()
            .any(|message| message == "Warning: No teams available for selection.")
    );
}

#[test]
fn auth_selects_a_team_and_returns_the_team_jwt() {
    let fixture = PollFixture::new(
        scripted_transport(vec![
            Reply::Response(response(200, TEAM_CHOICE)),
            Reply::Response(response(200, r#"{"status":"ready","key":"jwt-team-2"}"#)),
        ]),
        Echo::default(),
    );
    let mut input = piped_input(vec!["2"]);

    let result = fixture
        .authenticate(&mut input)
        .expect("the auth")
        .expect("the result");

    assert_eq!(result.api_key.expose_secret(), "jwt-team-2");
    assert_eq!(result.user_id.as_deref(), Some("user-1"));
    assert_eq!(result.teams, vec!["team-1", "team-2"]);
    assert_eq!(result.team_id, None);
    assert_eq!(
        fixture.transport.calls()[1],
        PollCall::Get {
            url: format!("{SERVER}/sso/cli/poll/login-1?team_id=team-2"),
            headers: vec![(
                String::from("x-litellm-cli-poll-secret"),
                String::from(POLL_SECRET)
            )],
            timeout: 10,
        }
    );
    assert!(
        fixture
            .echo
            .recorded()
            .iter()
            .any(|message| message.contains("Successfully generated JWT for team: team-2"))
    );
}

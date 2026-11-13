use super::fixtures::{
    Echo, POLL_SECRET, PollCall, PollFixture, Reply, SERVER, piped_input, response,
    scripted_transport,
};

#[test]
fn headers_carry_the_poll_secret_on_every_poll() {
    let choice = r#"{"status":"ready","requires_team_selection":true,"teams":["team-1","team-2"],"team_details":[{"team_id":"team-1"},{"team_id":"team-2"}]}"#;
    let fixture = PollFixture::new(
        scripted_transport(vec![
            Reply::Response(response(200, choice)),
            Reply::Response(response(200, r#"{"status":"ready","key":"jwt-team-1"}"#)),
        ]),
        Echo::default(),
    );
    let mut input = piped_input(vec!["1"]);
    let headers = vec![(
        String::from("x-litellm-cli-poll-secret"),
        String::from(POLL_SECRET),
    )];

    fixture.authenticate(&mut input).expect("the auth");

    assert_eq!(
        fixture.transport.calls(),
        vec![
            PollCall::Get {
                url: format!("{SERVER}/sso/cli/poll/login-1"),
                headers: headers.clone(),
                timeout: 10,
            },
            PollCall::Get {
                url: format!("{SERVER}/sso/cli/poll/login-1?team_id=team-1"),
                headers,
                timeout: 10,
            },
        ]
    );
}

use super::fixtures::{
    Echo, POLL_SECRET, PollCall, RecordingSleeper, Reply, SERVER, authenticate, piped_input,
    response, scripted_transport,
};

#[test]
fn headers_carry_the_poll_secret_on_every_poll() {
    let echo = Echo::default();
    let sleeper = RecordingSleeper::default();
    let mut input = piped_input(vec!["1"]);
    let choice = r#"{"status":"ready","requires_team_selection":true,"teams":["team-1","team-2"],"team_details":[{"team_id":"team-1"},{"team_id":"team-2"}]}"#;
    let transport = scripted_transport(vec![
        Reply::Response(response(200, choice)),
        Reply::Response(response(200, r#"{"status":"ready","key":"jwt-team-1"}"#)),
    ]);
    let headers = vec![(
        String::from("x-litellm-cli-poll-secret"),
        String::from(POLL_SECRET),
    )];

    authenticate(&transport, &sleeper, &mut input, &echo).expect("the auth");

    assert_eq!(
        transport.calls(),
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

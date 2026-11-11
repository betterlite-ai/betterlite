#![cfg(test)]

use std::collections::VecDeque;
use std::net::TcpListener;
use std::sync::Mutex;

use console::Key;

use crate::commands::Client;
use crate::commands::tests::support::{Answer, TestServer, client, client_for};
use crate::sso_polling::terminal::SelectionInput;

use super::{
    AssignKey, AvailableTeams, ListTeams, TeamsCommand, TeamsError, TeamsPorts, run_teams_command,
};

struct ScriptedInput {
    interactive: bool,
    lines: VecDeque<String>,
    keys: VecDeque<Key>,
    lines_read: usize,
}

impl ScriptedInput {
    fn answering(lines: Vec<&str>) -> Self {
        Self {
            interactive: false,
            lines: lines.into_iter().map(String::from).collect(),
            keys: VecDeque::new(),
            lines_read: 0,
        }
    }

    fn quiet() -> Self {
        Self {
            interactive: true,
            lines: VecDeque::new(),
            keys: VecDeque::new(),
            lines_read: 0,
        }
    }
}

impl SelectionInput for ScriptedInput {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn read_key(&mut self) -> Option<Key> {
        self.keys.pop_front()
    }

    fn read_line(&mut self) -> Option<String> {
        self.lines_read = self.lines_read.saturating_add(1);
        self.lines.pop_front()
    }
}

fn run(
    command: TeamsCommand,
    server: &TestServer,
    input: &mut ScriptedInput,
    echoed: &Mutex<Vec<String>>,
) -> (Result<(), TeamsError>, String, String) {
    let api = client_for(server, Some("sk-test"));
    run_with(&api, command, input, echoed)
}

fn run_with(
    api: &Client,
    command: TeamsCommand,
    input: &mut ScriptedInput,
    echoed: &Mutex<Vec<String>>,
) -> (Result<(), TeamsError>, String, String) {
    let echo = |line: &str| echoed.lock().unwrap().push(line.to_owned());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let ports = TeamsPorts {
        client: api,
        terminal: input,
        echo: &echo,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let result = run_teams_command(command, ports);
    (
        result,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn echoed_lines(echoed: &Mutex<Vec<String>>) -> Vec<String> {
    echoed.lock().unwrap().clone()
}

fn assert_contains_all(text: &str, needles: &[&str]) {
    for needle in needles {
        assert!(text.contains(needle), "missing {needle} in {text}");
    }
}

#[test]
fn list_behavior_prints_the_available_teams_table() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"[{"team_alias":"Alpha","team_id":"t1","models":["m1","m2","m3","m4"],"max_budget":10.5},{"team_id":"t2"}]"#,
    )]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(TeamsCommand::List(ListTeams), &server, &mut input, &echoed);

    result.unwrap();
    assert_contains_all(
        &stdout,
        &[
            "Available Teams",
            "Index",
            "Team Alias",
            "Team ID",
            "Max Budget",
            "Role",
            "Alpha",
            "t1",
            "m1, m2, m3 (+1 more)",
            "$10.5",
            "All models",
            "Unlimited",
            "Member",
            "N/A",
        ],
    );
    assert!(stderr.is_empty(), "{stderr}");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /team/list HTTP/1.1");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
}

#[test]
fn list_behavior_shows_an_explicit_zero_budget_as_dollars() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"[{"team_alias":"","team_id":"t1","max_budget":0}]"#,
    )]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) =
        run(TeamsCommand::List(ListTeams), &server, &mut input, &echoed);

    result.unwrap();
    assert!(stdout.contains("$0"), "{stdout}");
    assert!(!stdout.contains("Unlimited"), "{stdout}");
    assert!(!stdout.contains("N/A"), "{stdout}");
}

#[test]
fn list_behavior_prints_no_teams_found_for_an_empty_list() {
    let server = TestServer::start(vec![Answer::json(200, "[]")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) =
        run(TeamsCommand::List(ListTeams), &server, &mut input, &echoed);

    result.unwrap();
    assert_eq!(stdout, "No teams found for your user.\n");
}

#[test]
fn available_behavior_prints_the_joinable_teams() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"[{"team_alias":"Joinable","team_id":"t9"}]"#,
    )]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::Available(AvailableTeams),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert!(stdout.contains("\nAvailable Teams to Join:\n"), "{stdout}");
    assert!(stdout.contains("Joinable"), "{stdout}");
    assert!(stdout.contains("t9"), "{stdout}");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /team/available HTTP/1.1");
}

#[test]
fn available_behavior_prints_no_available_teams_for_an_empty_list() {
    let server = TestServer::start(vec![Answer::json(200, "[]")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::Available(AvailableTeams),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert_eq!(stdout, "No available teams to join.\n");
}

#[test]
fn assign_key_behavior_uses_the_given_team_id() {
    let server = TestServer::start(vec![
        Answer::json(200, r#"{"updated":true}"#),
        Answer::json(200, r#"[{"team_id":"t1","models":["m1","m2"]}]"#),
    ]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::AssignKey(AssignKey {
            team_id: Some(String::from("t1")),
        }),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert!(
        stdout.contains("\nAssigning your key to team: t1"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Successfully assigned key to team: t1"),
        "{stdout}"
    );
    assert!(
        stdout.contains("You can now access models: m1, m2"),
        "{stdout}"
    );
    assert_eq!(input.lines_read, 0);
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /key/update HTTP/1.1");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(
        requests[0].body_text(),
        "{\"key\":\"sk-test\",\"team_id\":\"t1\"}"
    );
    assert_eq!(requests[1].request_line, "GET /team/list HTTP/1.1");
}

#[test]
fn assign_key_behavior_prints_all_models_when_the_team_has_none() {
    let server = TestServer::start(vec![
        Answer::json(200, "{}"),
        Answer::json(200, r#"[{"team_id":"t1","models":[]}]"#),
    ]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::AssignKey(AssignKey {
            team_id: Some(String::from("t1")),
        }),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert!(
        stdout.contains("You can now access all available models"),
        "{stdout}"
    );
}

#[test]
fn assign_key_behavior_prompts_when_the_team_id_is_absent() {
    let server = TestServer::start(vec![
        Answer::json(
            200,
            r#"[{"team_alias":"Alpha","team_id":"t1"},{"team_alias":"Beta","team_id":"t2"}]"#,
        ),
        Answer::json(200, "{}"),
        Answer::json(200, r#"[{"team_id":"t2"}]"#),
    ]);
    let mut input = ScriptedInput::answering(vec!["2"]);
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::AssignKey(AssignKey { team_id: None }),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert!(
        stdout.contains("Successfully assigned key to team: t2"),
        "{stdout}"
    );
    let lines = echoed_lines(&echoed);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Selected team: Beta (t2)")),
        "{lines:?}"
    );
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /team/list HTTP/1.1");
    assert_eq!(requests[1].request_line, "POST /key/update HTTP/1.1");
    assert_eq!(
        requests[1].body_text(),
        "{\"key\":\"sk-test\",\"team_id\":\"t2\"}"
    );
}

#[test]
fn assign_key_behavior_does_not_prompt_for_an_explicit_empty_team_id() {
    let server = TestServer::start(vec![Answer::json(200, "{}"), Answer::json(200, "[]")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::AssignKey(AssignKey {
            team_id: Some(String::new()),
        }),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert!(
        stdout.contains("Successfully assigned key to team: "),
        "{stdout}"
    );
    assert_eq!(input.lines_read, 0);
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request_line, "POST /key/update HTTP/1.1");
    assert_eq!(
        requests[0].body_text(),
        "{\"key\":\"sk-test\",\"team_id\":\"\"}"
    );
}

#[test]
fn assign_key_behavior_requires_an_api_key() {
    let server = TestServer::start(Vec::new());
    let api = client(&server.base_url, None);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run_with(
        &api,
        TeamsCommand::AssignKey(AssignKey {
            team_id: Some(String::from("t1")),
        }),
        &mut input,
        &echoed,
    );

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(
        stdout.contains("No API key found. Please login first using 'lite login'"),
        "{stdout}"
    );
    assert!(server.finish().is_empty());
}

#[test]
fn assign_key_behavior_stops_when_the_user_has_no_teams() {
    let server = TestServer::start(vec![Answer::json(200, "[]")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::AssignKey(AssignKey { team_id: None }),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert_eq!(stdout, "No teams found for your user.\n");
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
}

#[test]
fn assign_key_behavior_prints_operation_cancelled_when_the_prompt_is_skipped() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"[{"team_alias":"Alpha","team_id":"t1"}]"#,
    )]);
    let mut input = ScriptedInput::answering(vec!["skip"]);
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, _stderr) = run(
        TeamsCommand::AssignKey(AssignKey { team_id: None }),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert_eq!(stdout, "Operation cancelled.\n");
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
}

#[test]
fn list_error_prints_the_status_then_a_json_error_body() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(TeamsCommand::List(ListTeams), &server, &mut input, &echoed);

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert_eq!(stdout, "{\"detail\":\"boom\"}\n");
    assert_eq!(stderr, "Error: HTTP 500\n");
}

#[test]
fn list_error_prints_a_non_json_error_body_as_raw_text() {
    let server = TestServer::start(vec![Answer::text(500, "the proxy broke")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(TeamsCommand::List(ListTeams), &server, &mut input, &echoed);

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(stderr, "Error: HTTP 500\nthe proxy broke\n");
}

#[test]
fn list_error_from_a_transport_failure_prints_one_line_and_aborts() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let api = client(&format!("http://{address}"), Some("sk-test"));
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) =
        run_with(&api, TeamsCommand::List(ListTeams), &mut input, &echoed);

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("Error: "), "{stderr}");
}

#[test]
fn list_error_from_an_unreadable_body_prints_one_line_and_aborts() {
    let server = TestServer::start(vec![Answer::text(200, "not json")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(TeamsCommand::List(ListTeams), &server, &mut input, &echoed);

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("Error: "), "{stderr}");
}

#[test]
fn available_error_prints_the_status_and_body_without_aborting() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(
        TeamsCommand::Available(AvailableTeams),
        &server,
        &mut input,
        &echoed,
    );

    result.unwrap();
    assert_eq!(stdout, "{\"detail\":\"boom\"}\n");
    assert_eq!(stderr, "Error: HTTP 500\n");
}

#[test]
fn available_error_from_an_unreadable_body_prints_one_line_and_aborts() {
    let server = TestServer::start(vec![Answer::text(200, "not json")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(
        TeamsCommand::Available(AvailableTeams),
        &server,
        &mut input,
        &echoed,
    );

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("Error: "), "{stderr}");
}

#[test]
fn assign_key_error_prints_the_status_and_body_of_a_failed_update() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(
        TeamsCommand::AssignKey(AssignKey {
            team_id: Some(String::from("t1")),
        }),
        &server,
        &mut input,
        &echoed,
    );

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.contains("{\"detail\":\"boom\"}"), "{stdout}");
    assert_eq!(stderr, "Error: HTTP 500\n");
}

#[test]
fn assign_key_error_from_an_unreadable_team_list_prints_one_line_and_aborts() {
    let server = TestServer::start(vec![Answer::text(200, "not json")]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(
        TeamsCommand::AssignKey(AssignKey { team_id: None }),
        &server,
        &mut input,
        &echoed,
    );

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.starts_with("Error: "), "{stderr}");
}

#[test]
fn list_error_prints_the_authentication_line_on_a_401() {
    let server = TestServer::start(vec![Answer::json(401, r#"{"detail":"unauthorized"}"#)]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(TeamsCommand::List(ListTeams), &server, &mut input, &echoed);

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr,
        "Error: Authentication failed. Check your API key.\n"
    );
}

#[test]
fn available_error_prints_the_authentication_line_and_aborts_on_a_401() {
    let server = TestServer::start(vec![Answer::json(401, r#"{"detail":"unauthorized"}"#)]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(
        TeamsCommand::Available(AvailableTeams),
        &server,
        &mut input,
        &echoed,
    );

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr,
        "Error: Authentication failed. Check your API key.\n"
    );
}

#[test]
fn assign_key_error_prints_the_authentication_line_on_a_401() {
    let server = TestServer::start(vec![Answer::json(401, r#"{"detail":"unauthorized"}"#)]);
    let mut input = ScriptedInput::quiet();
    let echoed = Mutex::new(Vec::new());

    let (result, stdout, stderr) = run(
        TeamsCommand::AssignKey(AssignKey { team_id: None }),
        &server,
        &mut input,
        &echoed,
    );

    assert!(matches!(result, Err(TeamsError::Aborted)));
    assert!(stdout.is_empty(), "{stdout}");
    assert_eq!(
        stderr,
        "Error: Authentication failed. Check your API key.\n"
    );
}

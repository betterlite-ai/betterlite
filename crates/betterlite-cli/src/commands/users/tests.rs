#![cfg(test)]

use rust_decimal::Decimal;

use crate::commands::tests::support::{Answer, TestServer, client_for};

use super::{
    CreateUser, DeleteUser, GetUser, ListUsers, UsersCommand, UsersError, run_users_command,
};

fn run(command: UsersCommand, server: &TestServer) -> (Result<(), UsersError>, String) {
    let api = client_for(server, Some("sk-test"));
    let mut stdout = Vec::new();
    let result = run_users_command(command, &api, &mut stdout);
    (result, String::from_utf8(stdout).unwrap())
}

fn create(email: &str) -> CreateUser {
    CreateUser {
        email: email.to_owned(),
        role: String::from("internal_user"),
        alias: None,
        team: Vec::new(),
        max_budget: None,
    }
}

#[test]
fn list_users_prints_a_row_for_every_user() {
    let server = TestServer::start(vec![Answer::json(
        200,
        r#"{"users":[{"user_id":"u1","user_email":"a@example.com","user_role":"admin","teams":["t1","t2"]},{"user_id":"u2"}]}"#,
    )]);

    let (result, stdout) = run(UsersCommand::List(ListUsers), &server);

    result.unwrap();
    assert!(stdout.contains("Users"), "{stdout}");
    assert!(stdout.contains("User ID"), "{stdout}");
    assert!(stdout.contains("Email"), "{stdout}");
    assert!(stdout.contains("Role"), "{stdout}");
    assert!(stdout.contains("Teams"), "{stdout}");
    assert!(stdout.contains("u1"), "{stdout}");
    assert!(stdout.contains("a@example.com"), "{stdout}");
    assert!(stdout.contains("t1, t2"), "{stdout}");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /user/list HTTP/1.1");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
}

#[test]
fn list_users_prints_no_users_found_for_an_empty_list() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"users":[]}"#)]);

    let (result, stdout) = run(UsersCommand::List(ListUsers), &server);

    result.unwrap();
    assert_eq!(stdout, "No users found.\n");
}

#[test]
fn list_users_prints_no_users_found_for_a_null_list() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"users":null}"#)]);

    let (result, stdout) = run(UsersCommand::List(ListUsers), &server);

    result.unwrap();
    assert_eq!(stdout, "No users found.\n");
}

#[test]
fn list_users_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(500, r#"{"detail":"boom"}"#)]);

    let (result, stdout) = run(UsersCommand::List(ListUsers), &server);

    assert!(matches!(result, Err(UsersError::Http { status: 500 })));
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn list_users_rejects_a_response_without_a_users_key() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, stdout) = run(UsersCommand::List(ListUsers), &server);

    assert!(matches!(result, Err(UsersError::Response { .. })));
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn list_users_rejects_a_non_list_users_value() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"users":"not-a-list"}"#)]);

    let (result, stdout) = run(UsersCommand::List(ListUsers), &server);

    assert!(matches!(result, Err(UsersError::Response { .. })));
    assert!(stdout.is_empty(), "{stdout}");
}

#[test]
fn get_user_sends_the_id_and_prints_the_response() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"user_id":"u1"}"#)]);

    let (result, stdout) = run(
        UsersCommand::Get(GetUser {
            user_id: Some(String::from("u1")),
        }),
        &server,
    );

    result.unwrap();
    assert_eq!(stdout, "{\"user_id\":\"u1\"}\n");
    let requests = server.finish();
    assert_eq!(
        requests[0].request_line,
        "GET /user/info?user_id=u1 HTTP/1.1"
    );
}

#[test]
fn get_user_omits_the_id_when_no_flag_is_given() {
    let server = TestServer::start(vec![Answer::json(200, "{}")]);

    let (result, _stdout) = run(UsersCommand::Get(GetUser { user_id: None }), &server);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /user/info HTTP/1.1");
}

#[test]
fn get_user_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(404, r#"{"detail":"missing"}"#)]);

    let (result, _stdout) = run(
        UsersCommand::Get(GetUser {
            user_id: Some(String::from("u1")),
        }),
        &server,
    );

    assert!(matches!(result, Err(UsersError::Http { status: 404 })));
}

#[test]
fn create_user_omits_an_absent_alias_and_absent_teams() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"user_id":"u3"}"#)]);

    let (result, stdout) = run(UsersCommand::Create(create("new@example.com")), &server);

    result.unwrap();
    assert_eq!(stdout, "{\"user_id\":\"u3\"}\n");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /user/new HTTP/1.1");
    assert_eq!(
        requests[0].body_text(),
        "{\"user_email\":\"new@example.com\",\"user_role\":\"internal_user\"}"
    );
}

#[test]
fn create_user_sends_a_given_empty_alias_a_team_list_and_a_zero_budget() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"user_id":"u3"}"#)]);

    let (result, _stdout) = run(
        UsersCommand::Create(CreateUser {
            email: String::from("new@example.com"),
            role: String::from("admin"),
            alias: Some(String::new()),
            team: vec![String::from("t1")],
            max_budget: Some(Decimal::ZERO),
        }),
        &server,
    );

    result.unwrap();
    let requests = server.finish();
    assert_eq!(
        requests[0].body_text(),
        "{\"user_email\":\"new@example.com\",\"user_role\":\"admin\",\"user_alias\":\"\",\"teams\":[\"t1\"],\"max_budget\":0}"
    );
}

#[test]
fn create_user_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(422, r#"{"detail":"bad"}"#)]);

    let (result, _stdout) = run(UsersCommand::Create(create("new@example.com")), &server);

    assert!(matches!(result, Err(UsersError::Http { status: 422 })));
}

#[test]
fn delete_user_sends_every_id_in_one_request() {
    let server = TestServer::start(vec![Answer::json(200, r#"{"deleted":2}"#)]);

    let (result, stdout) = run(
        UsersCommand::Delete(DeleteUser {
            user_ids: vec![String::from("u1"), String::from("u2")],
        }),
        &server,
    );

    result.unwrap();
    assert_eq!(stdout, "{\"deleted\":2}\n");
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "POST /user/delete HTTP/1.1");
    assert_eq!(requests[0].body_text(), "{\"user_ids\":[\"u1\",\"u2\"]}");
}

#[test]
fn delete_user_http_error_is_an_error_without_its_own_handler() {
    let server = TestServer::start(vec![Answer::json(403, r#"{"detail":"no"}"#)]);

    let (result, _stdout) = run(
        UsersCommand::Delete(DeleteUser {
            user_ids: vec![String::from("u1")],
        }),
        &server,
    );

    assert!(matches!(result, Err(UsersError::Http { status: 403 })));
}

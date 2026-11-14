use crate::commands::chat::Message;
use crate::commands::tests::support::Answer;
use crate::commands::tests::support::TestServer;

use super::run_once;
use super::support::FakeStore;
use super::support::chat_args;
use super::support::stream_body;

#[test]
fn session_welcome_states_the_model_temperature_and_unlimited_tokens() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &[], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("LiteLLM Interactive Chat"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("Model: gpt-4o"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("Temperature: 0.7"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("Max Tokens: unlimited"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal
            .stdout
            .contains("Type your messages and press Enter."),
        "{}",
        terminal.stdout
    );
}

#[test]
fn session_welcome_shows_an_explicit_zero_max_tokens() {
    let server = TestServer::start(Vec::new());
    let mut args = chat_args(Some("gpt-4o"));
    args.max_tokens = Some(0);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &[], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Max Tokens: 0"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn session_end_of_input_prints_chat_session_ended() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &[], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.ends_with("You: \nChat session ended.\n"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn session_blank_input_is_not_sent() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["", "   "], &mut store);

    result.unwrap();
    assert!(
        !terminal.stdout.contains("Assistant:"),
        "{}",
        terminal.stdout
    );
    assert_eq!(terminal.stdout.matches("You: ").count(), 3);
}

#[test]
fn session_history_labels_each_role() {
    let server = TestServer::start(Vec::new());
    let mut args = chat_args(Some("gpt-4o"));
    args.system = Some(String::from("be nice"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["/history"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("1. System: be nice"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn session_history_truncates_a_long_assistant_reply() {
    let server = TestServer::start(vec![Answer::json(
        200,
        &stream_body(&["a".repeat(120).as_str()]),
    )]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi", "/history"], &mut store);

    result.unwrap();
    let preview = format!("2. Assistant: {}...", "a".repeat(100));
    assert!(terminal.stdout.contains(&preview), "{}", terminal.stdout);
}

#[test]
fn session_history_numbering_counts_unknown_roles() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();
    let _files = store.files.insert(
        String::from("mixed.json"),
        vec![
            Message {
                role: String::from("user"),
                content: String::from("question"),
            },
            Message {
                role: String::from("tool"),
                content: String::from("result"),
            },
            Message {
                role: String::from("assistant"),
                content: String::from("answer"),
            },
        ],
    );

    let (result, terminal) = run_once(&args, &server, &["/load mixed", "/history"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("1. You: question"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("3. Assistant: answer"),
        "{}",
        terminal.stdout
    );
    assert!(!terminal.stdout.contains("2."), "{}", terminal.stdout);
}

#[test]
fn session_assistant_reply_is_kept_for_history() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi", "/history"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("2. Assistant: Hello"),
        "{}",
        terminal.stdout
    );
}

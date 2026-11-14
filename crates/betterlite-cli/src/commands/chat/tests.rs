#![cfg(test)]

mod support;

mod models_cases;

mod session_cases;

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;

use rstest::rstest;
use serde::Deserialize;

use crate::commands::BODY_LIMIT_BYTES;
use crate::commands::chat::stream_line::{
    LineEvent, MAX_LINE_BYTES, StreamLineError, StreamLines, decode_line,
};
use crate::commands::tests::support::{
    Answer, RecordedRequest, TestServer, client as plain_client,
};

use super::{ChatArgs, Message};
use support::{FakeStore, FakeTerminal, chat_args, client, run_chat, stream_body};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedChat {
    model: String,
    messages: Vec<RecordedMessage>,
    stream: bool,
    temperature: f64,
    #[serde(default)]
    max_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct RecordedMessage {
    role: String,
    content: String,
}

fn run_once(
    args: &ChatArgs,
    server: &TestServer,
    input: &[&str],
    store: &mut FakeStore,
) -> (Result<(), super::ChatError>, FakeTerminal) {
    let api = client(server);
    let mut terminal = FakeTerminal::new(input);
    let result = run_chat(args, &api, &mut terminal, store);
    (result, terminal)
}

fn chat_body(requests: &[RecordedRequest]) -> RecordedChat {
    let chat = requests
        .iter()
        .find(|request| request.path() == "/chat/completions")
        .expect("a chat completion request");
    serde_json::from_slice(&chat.body).expect("the chat body is JSON")
}

#[rstest]
#[case::help("/help     - Show this help message")]
#[case::quit("/quit     - Exit the chat session (also /exit, /q)")]
#[case::clear("/clear    - Clear conversation history")]
#[case::history("/history  - Show conversation history")]
#[case::model("/model    - Switch to a different model")]
#[case::save("/save <name> - Save conversation to file")]
#[case::load("/load <name> - Load conversation from file")]
fn commands_help_lists_each_command(#[case] command: &str) {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["/help"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Interactive Chat Commands:"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains(command),
        "{command} is missing from {}",
        terminal.stdout
    );
}

#[test]
fn commands_clear_keeps_the_system_message() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let mut args = chat_args(Some("gpt-4o"));
    args.system = Some(String::from("be nice"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi", "/clear", "/history"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Conversation history cleared."),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("1. System: be nice"),
        "{}",
        terminal.stdout
    );
    assert!(!terminal.stdout.contains("2. You"), "{}", terminal.stdout);
}

#[test]
fn commands_save_and_load_round_trip() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(
        &args,
        &server,
        &["hi", "/save chat", "/load chat", "/history"],
        &mut store,
    );

    result.unwrap();
    assert_eq!(store.saved.len(), 1);
    assert_eq!(store.saved[0].0, "chat.json");
    assert_eq!(store.saved[0].1.len(), 2);
    assert!(
        terminal.stdout.contains("Conversation saved to chat.json"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal
            .stdout
            .contains("Conversation loaded from chat.json"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("1. You: hi"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("2. Assistant: Hello"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn commands_save_uses_only_the_first_two_words() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, _terminal) = run_once(&args, &server, &["/save a b c"], &mut store);

    result.unwrap();
    assert_eq!(store.saved.len(), 1);
    assert_eq!(store.saved[0].0, "a.json");
}

#[test]
fn commands_save_without_a_name_prints_usage_and_saves_nothing() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["/save"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Usage: /save <filename>"),
        "{}",
        terminal.stdout
    );
    assert!(store.saved.is_empty());
}

#[test]
fn commands_load_without_a_name_keeps_the_history() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi", "/load", "/history"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Usage: /load <filename>"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("1. You: hi"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn commands_load_missing_file_prints_not_found_and_falls_back() {
    let server = TestServer::start(Vec::new());
    let mut args = chat_args(Some("gpt-4o"));
    args.system = Some(String::from("be nice"));
    let mut store = FakeStore::default();
    store.files.insert(
        String::from("saved.json"),
        vec![Message {
            role: String::from("user"),
            content: String::from("old"),
        }],
    );

    let (result, terminal) = run_once(&args, &server, &["/load missing", "/history"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("File not found: missing.json"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("1. System: be nice"),
        "{}",
        terminal.stdout
    );
    assert!(!terminal.stdout.contains("old"), "{}", terminal.stdout);
}

#[test]
fn commands_load_failure_prints_the_error_and_falls_back() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();
    let _marked = store.unreadable.insert(String::from("broken.json"));

    let (result, terminal) = run_once(&args, &server, &["/load broken"], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stdout
            .contains("Error loading conversation: broken.json could not be read"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn commands_save_failure_prints_the_error() {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();
    let _marked = store.unwritable.insert(String::from("chat.json"));

    let (result, terminal) = run_once(&args, &server, &["/save chat"], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stdout
            .contains("Error saving conversation: chat.json could not be written"),
        "{}",
        terminal.stdout
    );
}

#[rstest]
#[case::quit("/quit")]
#[case::exit("/exit")]
#[case::q("/q")]
fn commands_quit_variants_end_the_session(#[case] command: &str) {
    let server = TestServer::start(Vec::new());
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &[command, "hi"], &mut store);

    result.unwrap();
    assert_eq!(
        terminal.stdout.matches("Chat session ended.").count(),
        1,
        "{}",
        terminal.stdout
    );
    assert!(
        !terminal.stdout.contains("Assistant:"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn commands_unrecognized_prefix_is_dropped_instead_of_sent() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["/quitnow", "hi"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Assistant:"),
        "{}",
        terminal.stdout
    );
    assert!(!terminal.stdout.contains("Usage:"), "{}", terminal.stdout);
    let requests = server.finish();
    let body = chat_body(&requests);
    assert_eq!(body.messages.len(), 1);
    assert_eq!(body.messages[0].content, "hi");
}

#[test]
fn commands_model_switch_cancelled_stays_silent() {
    let server = TestServer::start(vec![Answer::json(
        200,
        &support::models_body(&[("gpt-4o", "openai")]),
    )]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["/model"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Model selection cancelled."),
        "{}",
        terminal.stdout
    );
    assert!(
        !terminal.stdout.contains("Switched to model"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("Chat session ended."),
        "{}",
        terminal.stdout
    );
}

#[test]
fn commands_model_switch_updates_the_model() {
    let server = TestServer::start(vec![
        Answer::json(
            200,
            &support::models_body(&[("gpt-4o", "openai"), ("claude", "anthropic")]),
        ),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["/model", "2", "hi"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Switched to model: claude"),
        "{}",
        terminal.stdout
    );
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "claude");
}

#[test]
fn commands_model_switch_ignores_an_empty_id() {
    let server = TestServer::start(vec![
        Answer::json(200, &support::models_body(&[("", "openai")])),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["/model", "", "hi"], &mut store);

    result.unwrap();
    assert!(
        !terminal.stdout.contains("Switched to model"),
        "{}",
        terminal.stdout
    );
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "gpt-4o");
}

#[test]
fn stream_sends_the_completion_request_fields() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let mut args = chat_args(Some("gpt-4o"));
    args.temperature = 0.5_f64;
    args.max_tokens = Some(128);
    args.system = Some(String::from("be nice"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi", "/quit"], &mut store);

    result.unwrap();
    assert!(terminal.stderr.is_empty(), "{}", terminal.stderr);
    let requests = server.finish();
    let chat = requests
        .iter()
        .find(|request| request.path() == "/chat/completions")
        .expect("a chat completion request");
    assert_eq!(chat.method(), "POST");
    assert_eq!(chat.header("content-type"), Some("application/json"));
    assert_eq!(chat.header("authorization"), Some("Bearer sk-test"));
    let body: RecordedChat = serde_json::from_slice(&chat.body).expect("the chat body");
    assert_eq!(body.model, "gpt-4o");
    assert!(body.stream);
    assert_eq!(body.temperature.to_bits(), 0.5_f64.to_bits());
    assert_eq!(body.max_tokens, Some(128));
    assert_eq!(body.messages.len(), 2);
    assert_eq!(body.messages[0].role, "system");
    assert_eq!(body.messages[0].content, "be nice");
    assert_eq!(body.messages[1].role, "user");
    assert_eq!(body.messages[1].content, "hi");
}

#[test]
fn stream_omits_max_tokens_when_it_is_not_given() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, _terminal) = run_once(&args, &server, &["hi", "/quit"], &mut store);

    result.unwrap();
    let requests = server.finish();
    let chat = requests
        .iter()
        .find(|request| request.path() == "/chat/completions")
        .expect("a chat completion request");
    assert!(
        !chat.body_text().contains("max_tokens"),
        "{}",
        chat.body_text()
    );
    let body: RecordedChat = serde_json::from_slice(&chat.body).expect("the chat body");
    assert_eq!(body.max_tokens, None);
    assert_eq!(body.temperature.to_bits(), 0.7_f64.to_bits());
}

#[test]
fn stream_writes_the_deltas_as_fragments_and_stops_at_done() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"He\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\ndata: [DONE]\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"after\"}}]}\n";
    let server = TestServer::start(vec![Answer::json(200, body)]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Assistant:\nHello\n"),
        "{}",
        terminal.stdout
    );
    assert!(!terminal.stdout.contains("after"), "{}", terminal.stdout);
}

#[test]
fn stream_prints_one_newline_after_a_reply() {
    let server = TestServer::start(vec![Answer::json(200, &stream_body(&["Hello"]))]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi"], &mut store);

    result.unwrap();
    assert_eq!(
        terminal.stdout,
        "LiteLLM Interactive Chat\n\
         Model: gpt-4o\n\
         Temperature: 0.7\n\
         Max Tokens: unlimited\n\
         \n\
         Type your messages and press Enter. Type '/quit' or '/exit' to end the session.\n\
         Type '/help' for more commands.\n\
         \n\
         You: Assistant:\n\
         Hello\n\
         \n\
         You: \n\
         Chat session ended.\n"
    );
}

#[test]
fn stream_reassembles_a_line_split_across_chunks() {
    let mut lines = StreamLines::default();

    let first = lines.push(b"data: {\"choi").unwrap();
    assert!(first.is_empty());
    let second = lines
        .push(b"ces\":[{\"delta\":{\"content\":\"ok\"}}]}\n")
        .unwrap();

    assert_eq!(second, vec![LineEvent::Content(String::from("ok"))]);
}

#[test]
fn stream_reassembles_a_prefix_split_across_chunks() {
    let mut lines = StreamLines::default();

    let first = lines.push(b"da").unwrap();
    assert!(first.is_empty());
    let second = lines.push(b"ta: {\"choices\":[]}\n").unwrap();

    assert_eq!(second, vec![LineEvent::Ignored]);
}

#[test]
fn stream_skips_empty_and_non_data_lines_and_bad_json() {
    let mut lines = StreamLines::default();

    let events = lines
        .push(b"\n: keep-alive\nevent: message\ndata: not json\n")
        .unwrap();

    assert_eq!(
        events,
        vec![
            LineEvent::Ignored,
            LineEvent::Ignored,
            LineEvent::Ignored,
            LineEvent::Ignored
        ]
    );
}

#[test]
fn stream_finishes_a_trailing_line_without_a_newline() {
    let mut lines = StreamLines::default();

    let events = lines
        .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"end\"}}]}")
        .unwrap();
    assert!(events.is_empty());
    let finished = lines.finish().unwrap();

    assert_eq!(finished, Some(LineEvent::Content(String::from("end"))));
}

#[rstest]
#[case::done(b"data: [DONE]", LineEvent::Done)]
#[case::trimmed_done(b"data:   [DONE]  ", LineEvent::Done)]
#[case::carriage_return(b"data: [DONE]\r", LineEvent::Done)]
fn stream_decodes_a_done_payload(#[case] line: &[u8], #[case] expected: LineEvent) {
    assert_eq!(decode_line(line).unwrap(), expected);
}

#[test]
fn stream_rejects_a_line_that_is_not_utf8() {
    let error = decode_line(&[b'd', b'a', b't', b'a', b':', b' ', 0xff, 0xfe]).unwrap_err();

    assert!(matches!(error, StreamLineError::NotUtf8), "{error:?}");
}

#[rstest]
#[case::at_the_cap(MAX_LINE_BYTES, true)]
#[case::past_the_cap(MAX_LINE_BYTES + 1, false)]
fn stream_caps_an_unterminated_line(#[case] size: usize, #[case] accepted: bool) {
    let mut lines = StreamLines::default();

    let result = lines.push(&vec![b'x'; size]);

    assert_eq!(result.is_ok(), accepted, "{result:?}");
    if !accepted {
        assert!(
            matches!(
                &result,
                Err(StreamLineError::TooLarge { limit }) if *limit == MAX_LINE_BYTES
            ),
            "{result:?}"
        );
    }
}

#[rstest]
#[case::at_the_cap(MAX_LINE_BYTES, true)]
#[case::past_the_cap(MAX_LINE_BYTES + 1, false)]
fn stream_caps_a_completed_line(#[case] size: usize, #[case] accepted: bool) {
    let mut lines = StreamLines::default();
    let mut chunk = vec![b'x'; size];
    chunk.push(b'\n');

    let result = lines.push(&chunk);

    assert_eq!(result.is_ok(), accepted, "{result:?}");
    if !accepted {
        assert!(
            matches!(
                &result,
                Err(StreamLineError::TooLarge { limit }) if *limit == MAX_LINE_BYTES
            ),
            "{result:?}"
        );
    }
}

#[test]
fn stream_reports_a_chunk_of_the_wrong_shape() {
    let body = String::from("data: [1,2]\n\n");
    let server = TestServer::start(vec![Answer::json(200, &body)]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi", "again"], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stderr
            .contains("Error: the stream chunk has a shape the proxy does not send"),
        "{}",
        terminal.stderr
    );
    assert_eq!(
        terminal
            .stdout
            .matches("Error: No content received from the model")
            .count(),
        2,
        "{}",
        terminal.stdout
    );
}

#[test]
fn stream_prints_the_http_status_and_the_error_message() {
    let server = TestServer::start(vec![Answer::json(
        429,
        r#"{"error":{"message":"slow down"}}"#,
    )]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi"], &mut store);

    result.unwrap();
    assert_eq!(terminal.stderr, "Error: HTTP 429\nslow down\n");
    assert!(
        terminal
            .stdout
            .contains("Error: No content received from the model"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn stream_reports_a_401_as_one_error_line() {
    let server = TestServer::start(vec![Answer::json(
        401,
        r#"{"error":{"message":"secret detail"}}"#,
    )]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi"], &mut store);

    result.unwrap();
    assert_eq!(terminal.stderr, "Error: the proxy answered HTTP 401\n");
    assert!(
        terminal
            .stdout
            .contains("Error: No content received from the model"),
        "{}",
        terminal.stdout
    );
    assert!(
        !terminal.stdout.contains("secret detail"),
        "{}",
        terminal.stdout
    );
}

#[test]
fn stream_defaults_to_unknown_error_without_the_message_field() {
    let server = TestServer::start(vec![Answer::json(500, "{}")]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi"], &mut store);

    result.unwrap();
    assert_eq!(terminal.stderr, "Error: HTTP 500\nUnknown error\n");
}

#[test]
fn stream_prints_the_raw_body_when_it_is_not_json() {
    let server = TestServer::start(vec![Answer::text(502, "upstream down")]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi"], &mut store);

    result.unwrap();
    assert_eq!(terminal.stderr, "Error: HTTP 502\nupstream down\n");
}

#[test]
fn stream_reports_a_transport_failure_and_keeps_the_session() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let api = plain_client(&format!("http://{address}"), None);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();
    let mut terminal = FakeTerminal::new(&["hi", "again"]);

    let result = run_chat(&args, &api, &mut terminal, &mut store);

    result.unwrap();
    assert!(
        terminal.stderr.contains("Error: the request failed"),
        "{}",
        terminal.stderr
    );
    assert_eq!(
        terminal
            .stdout
            .matches("Error: No content received from the model")
            .count(),
        2,
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("Chat session ended."),
        "{}",
        terminal.stdout
    );
}

#[test]
fn stream_reports_response_headers_past_the_limit() {
    let oversized = "x".repeat(70000);
    let server = TestServer::start(vec![
        Answer::json(200, &stream_body(&["Hello"])).with_header("x-oversized", &oversized),
    ]);
    let args = chat_args(Some("gpt-4o"));
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["hi"], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stderr
            .contains("Error: the response headers are larger than 65536 bytes"),
        "{}",
        terminal.stderr
    );
}

#[rstest]
#[case::at_the_cap(0, false)]
#[case::past_the_cap(1, true)]
fn stream_caps_the_collected_reply(#[case] extra: usize, #[case] rejected: bool) {
    let mut terminal = FakeTerminal::new(&[]);
    let mut collected = "x".repeat(BODY_LIMIT_BYTES.saturating_add(extra));
    let events = vec![LineEvent::Content(String::new())];

    let result = super::stream::write_events(&mut terminal, events, &mut collected);

    assert_eq!(result.is_err(), rejected, "{result:?}");
    if rejected {
        assert!(
            matches!(
                result,
                Err(super::ChatError::ReplyTooLarge { limit }) if limit == BODY_LIMIT_BYTES
            ),
            "{result:?}"
        );
    }
}

#[rstest]
fn replays_the_stream_line_corpus(#[files("fuzz/corpus/stream_line/*")] path: PathBuf) {
    let Ok(bytes) = fs::read(&path) else {
        panic!("{}", path.display());
    };
    let _event = decode_line(&bytes);
}

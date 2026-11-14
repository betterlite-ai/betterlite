use rstest::rstest;

use crate::commands::tests::support::Answer;
use crate::commands::tests::support::TestServer;

use super::chat_body;
use super::run_once;
use super::support;
use super::support::FakeStore;
use super::support::chat_args;
use super::support::server_with_models;
use super::support::stream_body;

#[test]
fn models_picker_shows_the_table_and_picks_by_index() {
    let server = TestServer::start(vec![
        Answer::json(
            200,
            &support::models_body(&[("gpt-4o", "openai"), ("claude", "anthropic")]),
        ),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["2", "hi"], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Available Models"),
        "{}",
        terminal.stdout
    );
    assert!(terminal.stdout.contains("Model ID"), "{}", terminal.stdout);
    assert!(terminal.stdout.contains("Owned By"), "{}", terminal.stdout);
    assert!(terminal.stdout.contains("gpt-4o"), "{}", terminal.stdout);
    assert!(terminal.stdout.contains("anthropic"), "{}", terminal.stdout);
    assert!(
        !terminal.stdout.contains("more models"),
        "{}",
        terminal.stdout
    );
    let requests = server.finish();
    assert_eq!(requests[0].request_line, "GET /models HTTP/1.1");
    assert_eq!(requests[0].header("authorization"), Some("Bearer sk-test"));
    let body = chat_body(&requests);
    assert_eq!(body.model, "claude");
}

#[test]
fn models_blank_answer_picks_the_first_model() {
    let server = TestServer::start(vec![
        Answer::json(
            200,
            &support::models_body(&[("gpt-4o", "openai"), ("claude", "anthropic")]),
        ),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, _terminal) = run_once(&args, &server, &["", "hi"], &mut store);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "gpt-4o");
}

#[test]
fn models_out_of_range_index_reprompts_with_the_valid_range() {
    let server = TestServer::start(vec![
        Answer::json(
            200,
            &support::models_body(&[("gpt-4o", "openai"), ("claude", "anthropic")]),
        ),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["9", "1", "hi"], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stdout
            .contains("Invalid index. Please enter a number between 1 and 2"),
        "{}",
        terminal.stdout
    );
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "gpt-4o");
}

#[test]
fn models_type_a_name_uses_it_directly() {
    let server = TestServer::start(vec![
        Answer::json(200, &support::models_body(&[("gpt-4o", "openai")])),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, _terminal) = run_once(&args, &server, &["my-model", "hi"], &mut store);

    result.unwrap();
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "my-model");
}

#[test]
fn models_a_model_without_an_id_is_no_selection() {
    let server = server_with_models(&[("", "openai")]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &[""], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("No model selected. Exiting."),
        "{}",
        terminal.stdout
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn models_whitespace_choice_reprompts_for_a_name() {
    let server = TestServer::start(vec![
        Answer::json(200, &support::models_body(&[("gpt-4o", "openai")])),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["   ", "1", "hi"], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stdout
            .contains("Please enter a valid model name or index"),
        "{}",
        terminal.stdout
    );
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "gpt-4o");
}

#[test]
fn models_skips_an_entry_that_is_not_an_object() {
    let server = TestServer::start(vec![
        Answer::json(200, r#"{"data":[7,{"id":"gpt-4o","owned_by":"openai"}]}"#),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["1", "hi"], &mut store);

    result.unwrap();
    assert!(terminal.stderr.is_empty(), "{}", terminal.stderr);
    assert!(terminal.stdout.contains("gpt-4o"), "{}", terminal.stdout);
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "gpt-4o");
}

#[test]
fn models_end_of_input_cancels_the_selection() {
    let server = server_with_models(&[("gpt-4o", "openai")]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &[], &mut store);

    result.unwrap();
    assert!(
        terminal.stdout.contains("Model selection cancelled."),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("No model selected. Exiting."),
        "{}",
        terminal.stdout
    );
    assert_eq!(server.finish().len(), 1);
}

#[rstest]
#[case::blank("")]
#[case::whitespace("   ")]
fn models_blank_name_prompt_returns_no_model(#[case] answer: &str) {
    let server = TestServer::start(vec![Answer::json(200, r#"{"data":[]}"#)]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &[answer], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stdout
            .contains("No models available or could not fetch models list."),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("Please enter a model name"),
        "{}",
        terminal.stdout
    );
    assert!(
        terminal.stdout.contains("No model selected. Exiting."),
        "{}",
        terminal.stdout
    );
}

#[test]
fn models_fetch_failure_prints_a_warning_and_asks_for_a_name() {
    let server = TestServer::start(vec![
        Answer::json(500, r#"{"detail":"boom"}"#),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["gpt-4o", "hi"], &mut store);

    result.unwrap();
    assert!(
        terminal
            .stderr
            .contains("Warning: Could not fetch models list: the proxy answered HTTP 500"),
        "{}",
        terminal.stderr
    );
    let requests = server.finish();
    assert_eq!(chat_body(&requests).model, "gpt-4o");
}

#[test]
fn models_missing_data_field_prints_a_warning() {
    let server = TestServer::start(vec![
        Answer::json(200, "{}"),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["gpt-4o", "hi"], &mut store);

    result.unwrap();
    assert!(
        terminal.stderr.contains(
            "Warning: Could not fetch models list: the models response has no data field"
        ),
        "{}",
        terminal.stderr
    );
}

#[rstest]
#[case::an_object(r#"{"data":{}}"#)]
#[case::a_string(r#"{"data":"text"}"#)]
#[case::a_number(r#"{"data":7}"#)]
#[case::a_bool(r#"{"data":true}"#)]
#[case::null(r#"{"data":null}"#)]
fn models_data_that_is_not_an_array_is_silently_empty(#[case] models: &str) {
    let server = TestServer::start(vec![
        Answer::json(200, models),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["gpt-4o", "hi"], &mut store);

    result.unwrap();
    assert!(terminal.stderr.is_empty(), "{}", terminal.stderr);
    assert!(
        terminal.stdout.contains("Please enter a model name"),
        "{}",
        terminal.stdout
    );
}

#[rstest]
#[case::at_the_cap(200, false)]
#[case::past_the_cap(205, true)]
fn models_table_caps_at_two_hundred_rows(#[case] count: usize, #[case] expects_more: bool) {
    let models: Vec<(String, String)> = (1..=count)
        .map(|index| (format!("m-{index}"), String::from("owner")))
        .collect();
    let entries: Vec<(&str, &str)> = models
        .iter()
        .map(|(id, owner)| (id.as_str(), owner.as_str()))
        .collect();
    let server = TestServer::start(vec![
        Answer::json(200, &support::models_body(&entries)),
        Answer::json(200, &stream_body(&["Hello"])),
    ]);
    let args = chat_args(None);
    let mut store = FakeStore::default();

    let (result, terminal) = run_once(&args, &server, &["1", "hi"], &mut store);

    result.unwrap();
    assert!(terminal.stdout.contains("m-200"), "{}", terminal.stdout);
    assert!(!terminal.stdout.contains("m-201"), "{}", terminal.stdout);
    assert_eq!(
        terminal.stdout.contains("more models"),
        expects_more,
        "{}",
        terminal.stdout
    );
    if expects_more {
        assert!(
            terminal.stdout.contains("... and 5 more models"),
            "{}",
            terminal.stdout
        );
    }
}

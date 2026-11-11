use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::logging_state::types::DynamicCallbacks;
use rstest::rstest;

use super::support::Outcome;
use super::support::chat_request;
use super::support::hook;
use super::support::labels;
use super::support::setup;
use super::support::setup_with;

fn name(label: &str) -> DynamicCallback {
    DynamicCallback::Name(label.to_owned())
}

#[test]
fn callback_request_entry_lands_in_all_five_lists() {
    let mut fixture = setup_with(&[("langfuse", Outcome::Logger(String::from("langfuse")))]);
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"callbacks":["langfuse"]}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(labels(&fixture.lists.input), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.success), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.async_success), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.failure), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.async_failure), ["logger:langfuse"]);
    assert!(result.request.get("callbacks").is_none());
}

#[test]
fn callback_repeated_request_entry_is_registered_once() {
    let mut fixture = setup_with(&[("langfuse", Outcome::Logger(String::from("langfuse")))]);
    fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"callbacks":["langfuse","langfuse"]}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(labels(&fixture.lists.input), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.success), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.async_success), ["logger:langfuse"]);
}

#[test]
fn callback_two_loggers_of_one_class_with_different_names_both_stay() {
    let mut fixture = setup();
    fixture.callbacks = vec![hook("otel"), hook("arize")];
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(
        labels(&fixture.lists.success),
        ["logger:otel", "logger:arize"]
    );
    assert_eq!(
        labels(&fixture.lists.async_success),
        ["logger:otel", "logger:arize"]
    );
    assert_eq!(
        labels(&fixture.lists.input),
        ["logger:otel", "logger:arize"]
    );
}

#[test]
fn callback_unknown_name_is_skipped() {
    let mut fixture = setup();
    fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"callbacks":["unknown"]}"#),
        )
        .expect("the setup succeeds");

    assert!(fixture.lists.input.is_empty());
    assert!(fixture.lists.success.is_empty());
    assert!(fixture.lists.failure.is_empty());
}

#[test]
fn callback_resolver_that_returns_nothing_is_skipped() {
    let mut fixture = setup_with(&[("absent", Outcome::Missing)]);
    fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"callbacks":["absent"]}"#),
        )
        .expect("the setup succeeds");

    assert!(fixture.lists.input.is_empty());
    assert!(fixture.lists.success.is_empty());
}

#[test]
fn callback_known_global_success_name_is_resolved() {
    let mut fixture = setup_with(&[("langfuse", Outcome::Logger(String::from("langfuse")))]);
    fixture.lists.success.push(name("langfuse"));
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(labels(&fixture.lists.success), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.async_success), ["logger:langfuse"]);
}

#[test]
fn callback_every_known_global_success_name_is_resolved() {
    let mut fixture = setup_with(&[
        ("first", Outcome::Logger(String::from("first"))),
        ("second", Outcome::Logger(String::from("second"))),
    ]);
    fixture.lists.success = vec![name("first"), name("custom"), name("second")];
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(
        labels(&fixture.lists.success),
        ["name:custom", "logger:first", "logger:second"]
    );
    assert_eq!(
        labels(&fixture.lists.async_success),
        ["logger:first", "logger:second"]
    );
}

#[test]
fn callback_known_global_success_name_stays_when_its_logger_is_registered() {
    let mut fixture = setup_with(&[("langfuse", Outcome::Logger(String::from("langfuse")))]);
    fixture.lists.success.push(name("langfuse"));
    fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"callbacks":["langfuse"]}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        labels(&fixture.lists.success),
        ["name:langfuse", "logger:langfuse"]
    );
}

#[test]
fn callback_known_global_failure_name_is_resolved() {
    let mut fixture = setup_with(&[("langfuse", Outcome::Logger(String::from("langfuse")))]);
    fixture.lists.failure.push(name("langfuse"));
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(labels(&fixture.lists.failure), ["logger:langfuse"]);
    assert_eq!(labels(&fixture.lists.async_failure), ["logger:langfuse"]);
    assert!(fixture.lists.success.is_empty());
}

#[rstest]
#[case::dynamodb("dynamodb")]
#[case::openmeter("openmeter")]
fn callback_async_success_name_moves_to_the_async_list(#[case] label: &str) {
    let mut fixture = setup();
    fixture.lists.success.push(name(label));
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert!(fixture.lists.success.is_empty());
    assert_eq!(
        labels(&fixture.lists.async_success),
        [format!("name:{label}")]
    );
}

#[test]
fn callback_success_request_list_moves_dynamodb_and_s3_to_the_async_list() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"success_callback":["dynamodb","s3","langfuse"]}"#),
        )
        .expect("the setup succeeds");

    let setup = result.logging.dynamic_callbacks;
    assert_eq!(
        setup,
        DynamicCallbacks {
            input: None,
            success: Some(vec![String::from("langfuse")]),
            async_success: Some(vec![String::from("dynamodb"), String::from("s3")]),
            failure: None,
            async_failure: None,
        }
    );
    assert!(result.request.get("success_callback").is_none());
}

#[test]
fn callback_failure_request_list_has_no_async_split() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"failure_callback":["dynamodb","langfuse"]}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.dynamic_callbacks.failure,
        Some(vec![String::from("dynamodb"), String::from("langfuse")])
    );
    assert!(result.logging.dynamic_callbacks.async_failure.is_none());
    assert!(result.request.get("failure_callback").is_none());
}

#[test]
fn callback_absent_request_lists_stay_absent() {
    let mut fixture = setup();
    let result = fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.dynamic_callbacks,
        DynamicCallbacks::default()
    );
}

#[test]
fn callback_non_list_success_callback_is_rejected() {
    let mut fixture = setup();
    let error = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"success_callback":"langfuse"}"#),
        )
        .expect_err("a text success_callback is outside the declared type");

    assert_eq!(
        error.to_string(),
        "the success_callback value must be a list of strings"
    );
}

#[test]
fn callback_global_success_name_is_kept_when_no_resolver_matches() {
    let mut fixture = setup();
    fixture.lists.success.push(name("custom"));
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(labels(&fixture.lists.success), ["name:custom"]);
}

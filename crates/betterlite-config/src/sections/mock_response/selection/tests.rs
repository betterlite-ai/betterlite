#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};

use super::{MockPermission, resolve_mock};
use crate::sections::mock_response::{MockFunctionCall, MockResponse, MockToolCall};

fn text(value: &str) -> MockResponse {
    MockResponse::Text(value.to_owned())
}

fn tool_calls() -> Vec<MockToolCall> {
    vec![MockToolCall {
        id: String::from("call-1"),
        kind: String::from("function"),
        function: MockFunctionCall {
            name: String::from("lookup"),
            arguments: String::from("{}"),
        },
    }]
}

fn allowed() -> MockPermission {
    MockPermission {
        allow_client_mock_response: true,
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_warnings<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    let guard: DefaultGuard = set_default(subscriber);
    let output = action();
    drop(guard);
    (output, captured.text())
}

#[test]
fn an_allowed_client_response_is_used() {
    let client = text("client");
    let plan = resolve_mock(None, allowed(), Some(&client), None, None).expect("mock");
    assert_eq!(plan.response, Some(&client));
}

#[test]
fn an_allowed_client_response_wins_over_the_operator_one() {
    let operator = text("operator");
    let client = text("client");
    let plan = resolve_mock(Some(&operator), allowed(), Some(&client), None, None).expect("mock");
    assert_eq!(plan.response, Some(&client));
}

#[test]
fn an_allowed_client_tool_call_is_used() {
    let calls = tool_calls();
    let plan = resolve_mock(None, allowed(), None, Some(&calls), None).expect("mock");
    assert_eq!(plan.tool_calls, Some(calls.as_slice()));
}

#[rstest]
#[case::response(Some(&text("client")), None, "mock_response")]
#[case::tool_calls(None, Some(&tool_calls()), "mock_tool_calls")]
fn a_denied_client_field_is_stripped_with_a_warning(
    #[case] response: Option<&MockResponse>,
    #[case] calls: Option<&Vec<MockToolCall>>,
    #[case] field: &str,
) {
    let (plan, warnings) = capture_warnings(|| {
        resolve_mock(
            None,
            MockPermission::default(),
            response,
            calls.map(Vec::as_slice),
            None,
        )
    });
    assert!(plan.is_none());
    assert!(warnings.contains(field), "{warnings}");
}

#[test]
fn a_denied_client_field_does_not_replace_the_operator_response() {
    let operator = text("operator");
    let client = text("client");
    let (resolved, warnings) = capture_warnings(|| {
        resolve_mock(
            Some(&operator),
            MockPermission::default(),
            Some(&client),
            None,
            None,
        )
    });
    let plan = resolved.expect("the operator response still mocks");
    assert_eq!(plan.response, Some(&operator));
    assert!(warnings.contains("mock_response"));
}

#[test]
fn no_mock_field_means_no_plan() {
    assert!(resolve_mock(None, allowed(), None, None, None).is_none());
}

#[test]
fn the_plan_carries_the_choice_count() {
    let client = text("client");
    let plan = resolve_mock(None, allowed(), Some(&client), None, Some(3)).expect("mock");
    assert_eq!(plan.choice_count, Some(3));
    assert_eq!(plan.prompt_tokens, None);
}

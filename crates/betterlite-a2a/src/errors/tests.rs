#![cfg(test)]

use std::error::Error as _;

use rstest::rstest;

use super::A2AError;
use super::LLM_PROVIDER;
use super::retries::RetryCounts;
use crate::card_resolver::fetch_error::{CardFetchError, ProbeFailure};

fn base_error(message: &str) -> A2AError {
    A2AError::Base {
        message: message.to_owned(),
        model: None,
        debug_info: None,
        retries: RetryCounts::default(),
    }
}

fn connection_error(message: &str) -> A2AError {
    A2AError::Connection {
        message: message.to_owned(),
        url: None,
        model: None,
        debug_info: None,
        retries: RetryCounts::default(),
    }
}

fn failure(path: &str, status: u16) -> ProbeFailure {
    ProbeFailure {
        path: path.to_owned(),
        reason: CardFetchError::Status { status },
    }
}

#[rstest]
#[case::base(base_error("boom"), 500)]
#[case::connection(connection_error("refused"), 503)]
#[case::agent_card(
    A2AError::AgentCard { message: "bad card".to_owned(), url: None, status_code: 404 },
    404
)]
#[case::agent_card_forbidden(
    A2AError::AgentCard { message: "bad card".to_owned(), url: None, status_code: 403 },
    403
)]
#[case::localhost(
    A2AError::LocalhostUrl {
        localhost_url: "http://localhost:8000/".to_owned(),
        base_url: "https://agent.example/".to_owned(),
        model: None,
        original_error: None,
    },
    503
)]
fn errors_status_codes_match_litellm(#[case] error: A2AError, #[case] expected: u16) {
    assert_eq!(error.status_code(), expected);
}

#[test]
fn errors_provider_name_is_a2a_agent() {
    assert_eq!(LLM_PROVIDER, "a2a_agent");
}

#[rstest]
#[case::none(None, None, "")]
#[case::retried(Some(2), None, " (retried 2 times)")]
#[case::max(None, Some(3), " (max retries 3)")]
#[case::both(Some(2), Some(3), " (retried 2 times, max retries 3)")]
#[case::explicit_zero(Some(0), None, " (retried 0 times)")]
fn errors_retry_counts_render_beside_the_message(
    #[case] retried: Option<u32>,
    #[case] max: Option<u32>,
    #[case] expected: &str,
) {
    assert_eq!(RetryCounts { retried, max }.render(), expected);
}

#[test]
fn errors_base_message_carries_retry_counts() {
    let error = A2AError::Base {
        message: "boom".to_owned(),
        model: None,
        debug_info: None,
        retries: RetryCounts {
            retried: Some(2),
            max: Some(3),
        },
    };
    assert_eq!(error.to_string(), "boom (retried 2 times, max retries 3)");
}

#[test]
fn errors_discovery_message_names_requested_url_and_every_attempt() {
    let error = A2AError::agent_card_discovery(
        "https://agent.example".to_owned(),
        vec![
            failure("/.well-known/agent-card.json", 404),
            failure("/.well-known/agent.json", 500),
        ],
    );
    assert_eq!(
        error.to_string(),
        "no agent card found at https://agent.example. Tried /.well-known/agent-card.json (HTTP 404), /.well-known/agent.json (HTTP 500)"
    );
    assert_eq!(error.status_code(), 500);
    assert_eq!(error.url(), Some("https://agent.example"));
}

#[test]
fn errors_discovery_status_is_first_non_404_or_404() {
    let error = A2AError::agent_card_discovery(
        "https://agent.example".to_owned(),
        vec![failure("/a", 404), failure("/b", 404)],
    );
    assert_eq!(error.status_code(), 404);
}

#[test]
fn errors_localhost_error_carries_both_urls_and_the_original_error() {
    let error = A2AError::LocalhostUrl {
        localhost_url: "http://localhost:8000/".to_owned(),
        base_url: "https://agent.example/".to_owned(),
        model: None,
        original_error: Some(Box::new(connection_error("refused"))),
    };
    assert_eq!(error.status_code(), 503);
    assert_eq!(error.url(), Some("http://localhost:8000/"));
    assert_eq!(
        error.to_string(),
        "the agent card points at the internal URL 'http://localhost:8000/'; retry with 'https://agent.example/'"
    );
    assert_eq!(
        error.source().map(ToString::to_string),
        Some("refused".to_owned())
    );
}

#[test]
fn errors_connection_error_reports_its_url() {
    let error = A2AError::Connection {
        message: "refused".to_owned(),
        url: Some("https://agent.example/".to_owned()),
        model: None,
        debug_info: None,
        retries: RetryCounts::default(),
    };
    assert_eq!(error.url(), Some("https://agent.example/"));
}

#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::agent_requests::error::AgentRequestError;
use crate::agent_requests::prepare::{Headers, PreparedAgentCall};
use crate::agent_results::error::AgentResultError;
use crate::agent_results::request::{StreamRequest, build_stream_request};
use crate::agent_results::stream::{open_stream, response_identity};

use super::support::{block_on, body_stream, object, setup, shown_headers, whole};

const FIRST_KEY: &str = "fixture-first-key-4c1d";
const SECOND_KEY: &str = "fixture-second-key-9a7b";

fn prepared(key: &str, agent: &str, base: &str, thread: Option<&str>) -> PreparedAgentCall {
    let headers: Headers = BTreeMap::from([(
        String::from("Authorization"),
        SecretString::from(format!("Bearer {key}")),
    )]);
    PreparedAgentCall {
        headers,
        api_version: String::from("2025-05-01"),
        agent_id: String::from(agent),
        thread_id: thread.map(str::to_owned),
        api_base: String::from(base),
    }
}

fn shown(request: &StreamRequest) -> String {
    let Some(secret) = request.headers.get("Authorization") else {
        return String::new();
    };
    secret.expose_secret().to_owned()
}

#[test]
fn agent_results_credential_identity_isolation() {
    let first = prepared(
        FIRST_KEY,
        "asst_first",
        "https://first.example/api",
        Some("thread-first"),
    );
    let second = prepared(
        SECOND_KEY,
        "asst_second",
        "https://second.example/api",
        None,
    );
    let empty = object("{}");

    let first_request = build_stream_request(&first, &[], &empty, &empty).expect("a request");
    let second_request = build_stream_request(&second, &[], &empty, &empty).expect("a request");

    assert_eq!(shown(&first_request), format!("Bearer {FIRST_KEY}"));
    assert_eq!(shown(&second_request), format!("Bearer {SECOND_KEY}"));
    assert_ne!(
        shown_headers(&first_request.headers),
        shown_headers(&second_request.headers)
    );
    assert_eq!(first_request.headers.len(), 1);
    assert_eq!(first_request.payload.assistant_id, "asst_first");
    assert_eq!(second_request.payload.assistant_id, "asst_second");
    assert!(
        first_request
            .url
            .starts_with("https://first.example/api/threads/runs")
    );
    assert!(
        second_request
            .url
            .starts_with("https://second.example/api/threads/runs")
    );
    assert!(!first_request.url.contains("second.example"));
    assert!(!second_request.url.contains("first.example"));
    assert!(first_request.payload.thread.is_none());
    assert_eq!(
        second_request
            .payload
            .thread
            .map(|thread| thread.messages.len()),
        Some(0)
    );

    let mut isolated = build_stream_request(&first, &[], &empty, &empty).expect("a request");
    isolated
        .headers
        .insert(String::from("X-Trace"), SecretString::from("trace"));
    let rebuilt = build_stream_request(&first, &[], &empty, &empty).expect("a request");
    assert!(!rebuilt.headers.contains_key("X-Trace"));
    assert_eq!(first.headers.len(), 1);
    assert_eq!(shown(&rebuilt), format!("Bearer {FIRST_KEY}"));

    let first_uuid = Uuid::from_u128(0x1a2b_3c4d_0000_4000_8000_0000_0000_0001);
    let second_uuid = Uuid::from_u128(0x9f8e_7d6c_0000_4000_8000_0000_0000_0002);
    let first_identity = response_identity(first_uuid, 1_000);
    assert_eq!(first_identity.id, "chatcmpl-1a2b3c4d");
    assert_eq!(first_identity, response_identity(first_uuid, 1_000));
    assert_ne!(first_identity, response_identity(second_uuid, 1_000));
    assert_ne!(first_identity, response_identity(first_uuid, 1_001));
    assert_eq!(first_identity.id.len(), "chatcmpl-".len().saturating_add(8));
}

#[test]
fn agent_results_secret_safe_failure() {
    let held = prepared(FIRST_KEY, "asst_first", "https://first.example/api", None);
    let empty = object("{}");
    let request = build_stream_request(&held, &[], &empty, &empty).expect("a request");

    assert!(!format!("{held:?}").contains(FIRST_KEY));
    assert!(!format!("{request:?}").contains(FIRST_KEY));
    assert_eq!(shown(&request), format!("Bearer {FIRST_KEY}"));

    let wrapped = AgentResultError::Request(AgentRequestError::Upstream {
        status: 401,
        message: String::from("denied"),
    });
    assert_eq!(wrapped.status(), Some(401));
    assert!(!format!("{wrapped:?}").contains(FIRST_KEY));
    assert!(!wrapped.to_string().contains(FIRST_KEY));

    block_on(async {
        let body_failure = open_stream(setup(401), whole("denied"))
            .await
            .err()
            .expect("a failure");
        assert_eq!(body_failure.to_string(), "Streaming request failed: denied");
        assert_eq!(body_failure.status(), Some(401));
        assert!(!format!("{body_failure:?}").contains(FIRST_KEY));

        let text_failure = open_stream(setup(502), body_stream(vec![vec![0xff]]))
            .await
            .err()
            .expect("a failure");
        assert_eq!(text_failure.status(), Some(502));
        assert!(!format!("{text_failure:?}").contains(FIRST_KEY));
    });
}

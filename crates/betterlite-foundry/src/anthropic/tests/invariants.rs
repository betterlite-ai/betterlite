use betterlite_http::client_kind::ClientKind;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::anthropic::request::Style;

use super::harness::{KeyShape, Run, Setup, execute};
use super::support::{CALLER_KEY, CONFIGURED_KEY, TOKEN, Trip, headers_of};

const FIRST_KEY: &str = "sk-ant-identity-one-0000000001";
const SECOND_KEY: &str = "sk-ant-identity-two-0000000002";
const FIRST_TOKEN: &str = "fixture-token-one-0000000001";
const SECOND_TOKEN: &str = "fixture-token-two-0000000002";

fn posted_header(run: &Run, name: &str) -> String {
    run.posted_headers[0]
        .get(name)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

fn identity_headers(identity: &'static str) -> IndexMap<String, String> {
    IndexMap::from([("x-identity".to_owned(), identity.to_owned())])
}

fn identity(
    headers: IndexMap<String, String>,
    caller: Option<&'static str>,
    configured: Option<&'static str>,
    token: Option<&'static str>,
) -> Run {
    execute(&Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Asynchronous,
        caller_headers: headers,
        argument_key: caller,
        configured_key: configured,
        token,
        ..Setup::chat()
    })
}

#[test]
fn anthropic_chat_credential_identity_isolation() {
    let first = identity(
        identity_headers("one"),
        Some(FIRST_KEY),
        Some(SECOND_KEY),
        Some(FIRST_TOKEN),
    );
    let second = identity(
        identity_headers("two"),
        None,
        Some(SECOND_KEY),
        Some(SECOND_TOKEN),
    );
    assert_identity_isolation(&first, &second);
    repeated_identity_is_stable();
}

fn assert_identity_isolation(first: &Run, second: &Run) {
    // each call sends its own argument key, and the configured key never overrides it
    assert_eq!(posted_header(first, "x-api-key"), FIRST_KEY);
    assert_eq!(posted_header(first, "api-key"), FIRST_KEY);
    assert!(first.posted_headers[0].get("authorization").is_none());
    // the second identity has no argument key, so only its configured key authenticates
    assert!(second.posted_headers[0].get("x-api-key").is_none());
    assert_eq!(posted_header(second, "api-key"), SECOND_KEY);
    assert_eq!(posted_header(first, "x-identity"), "one");
    assert_eq!(posted_header(second, "x-identity"), "two");
    // neither config sees the other call's identity, in headers or in the recorded request
    assert!(
        !first.transforms[0]
            .headers
            .values()
            .any(|value| value == SECOND_KEY)
    );
    assert!(
        !second.transforms[0]
            .headers
            .values()
            .any(|value| value == FIRST_KEY)
    );
    // the logging record of one identity holds neither identity's secrets
    assert!(!first.details_text().contains(FIRST_KEY));
    assert!(!first.details_text().contains(SECOND_KEY));
    assert!(!first.details_text().contains(FIRST_TOKEN));
    assert!(!second.details_text().contains(SECOND_TOKEN));
}

fn repeated_identity_is_stable() {
    let fresh = identity(
        identity_headers("one"),
        Some(FIRST_KEY),
        Some(SECOND_KEY),
        Some(FIRST_TOKEN),
    );
    assert_eq!(posted_header(&fresh, "x-api-key"), FIRST_KEY);
    assert_eq!(posted_header(&fresh, "api-key"), FIRST_KEY);
    assert_eq!(posted_header(&fresh, "x-identity"), "one");
    assert!(!fresh.details_text().contains(SECOND_TOKEN));
}

fn failure_headers() -> HeaderMap {
    headers_of(&[("request-id", "req-1")])
}

fn assert_secret_safe(run: &Run) {
    let rendered = [
        format!("{}", run.error()),
        format!("{:?}", run.error()),
        run.details_text(),
    ];
    for text in rendered {
        assert!(!text.contains(CALLER_KEY));
        assert!(!text.contains(CONFIGURED_KEY));
        assert!(!text.contains(TOKEN));
    }
}

#[test]
fn anthropic_chat_secret_safe_failure() {
    let rejected = execute(&Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Asynchronous,
        configured_key: Some(CONFIGURED_KEY),
        token: Some(TOKEN),
        trip: Trip::Status {
            status_code: 401,
            headers: failure_headers(),
            body: b"{\"error\":{\"message\":\"invalid api key\"}}".to_vec(),
            exception_headers: false,
        },
        ..Setup::chat()
    });
    assert_eq!(rejected.status(), Some(401));
    assert_secret_safe(&rejected);

    // a sync failure keeps the pre_call record, which masks the credential rather than dropping the header
    let refused = execute(&Setup {
        configured_key: Some(CONFIGURED_KEY),
        trip: Trip::Status {
            status_code: 401,
            headers: failure_headers(),
            body: b"{\"error\":{\"message\":\"invalid api key\"}}".to_vec(),
            exception_headers: false,
        },
        ..Setup::chat()
    });
    assert_eq!(refused.status(), Some(401));
    assert_secret_safe(&refused);
    let masked = refused
        .detail(&["additional_args", "headers", "x-api-key"])
        .unwrap();
    assert_ne!(masked, &Json::Text(CALLER_KEY.to_owned()));
    assert_early_failures_are_secret_safe();
}

fn assert_early_failures_are_secret_safe() {
    let unreachable = execute(&Setup {
        style: Style::Async,
        supplied_kind: ClientKind::Asynchronous,
        configured_key: Some(CONFIGURED_KEY),
        trip: Trip::Unknown("connect failed"),
        ..Setup::chat()
    });
    assert_secret_safe(&unreachable);

    let unprepared = execute(&Setup {
        key_shape: KeyShape::DictNull,
        configured_key: Some(CONFIGURED_KEY),
        transform_failure: Some("the config refused"),
        ..Setup::chat()
    });
    assert_secret_safe(&unprepared);

    // an offline token source fails with its own class and carries no credential into the message
    let untokened = execute(&Setup {
        argument_key: None,
        client_secret: Some("fixture-client-secret-value"),
        ..Setup::chat()
    });
    assert_secret_safe(&untokened);
}

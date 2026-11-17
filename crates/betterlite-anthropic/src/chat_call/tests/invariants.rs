use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::HeaderValue;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::credentials::AnthropicSettings;

use super::fixture::{CALLER_KEY, SETTINGS_KEY, Trip};
use super::harness::{Run, Setup, execute};

const FIRST_KEY: &str = "sk-ant-identity-one-0000000001";
const SECOND_KEY: &str = "sk-ant-identity-two-0000000002";

fn identity_headers(identity: &str) -> IndexMap<String, String> {
    let value = identity.to_owned();
    IndexMap::from([("x-identity".to_owned(), value)])
}

fn posted_header(run: &Run, name: &str) -> String {
    let value = run.posted_headers[0].get(name).unwrap();
    value.to_str().unwrap().to_owned()
}

fn identity(header: &str, caller: Option<&'static str>, settings_key: Option<&'static str>) -> Run {
    let mut setup = Setup::default();
    setup.acompletion = true;
    setup.caller_headers = identity_headers(header);
    setup.caller_key = caller;
    setup.settings_key = settings_key;
    execute(setup)
}

#[test]
fn chat_call_credential_identity_isolation() {
    let first = identity("one", Some(FIRST_KEY), Some(SECOND_KEY));
    let second = identity("two", Some(SECOND_KEY), Some(FIRST_KEY));
    // each call sends its own caller key, and the configured key never overrides it
    assert_eq!(posted_header(&first, "x-api-key"), FIRST_KEY);
    assert_eq!(posted_header(&second, "x-api-key"), SECOND_KEY);
    assert_eq!(posted_header(&first, "x-identity"), "one");
    assert_eq!(posted_header(&second, "x-identity"), "two");
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
    assert!(!first.details_text().contains(FIRST_KEY));
    assert!(!first.details_text().contains(SECOND_KEY));
    assert_settings_identity();
    assert_no_identity();
}

fn assert_settings_identity() {
    let mut setup = Setup::default();
    setup.acompletion = true;
    setup.caller_headers = IndexMap::new();
    setup.caller_key = None;
    setup.settings_key = Some(SECOND_KEY);
    let settings_only = execute(setup);
    assert_eq!(posted_header(&settings_only, "x-api-key"), SECOND_KEY);
    assert!(settings_only.posted_headers[0].get("x-identity").is_none());
    assert!(
        !settings_only.transforms[0]
            .headers
            .values()
            .any(|value| value == FIRST_KEY)
    );
}

fn assert_no_identity() {
    let mut setup = Setup::default();
    setup.acompletion = true;
    setup.caller_headers = identity_headers("three");
    setup.caller_key = None;
    setup.settings_key = None;
    let anonymous = execute(setup);
    assert_eq!(anonymous.status(), Some(401));
    assert!(anonymous.transforms.is_empty());
    assert_eq!(anonymous.posts_supplied + anonymous.posts_pooled, 0);
}

fn failure_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("request-id", HeaderValue::from_static("req-1"));
    headers
}

fn assert_secret_safe(run: &Run) {
    let rendered = [
        format!("{}", run.error()),
        format!("{:?}", run.error()),
        run.details_text(),
    ];
    for text in rendered {
        assert!(!text.contains(CALLER_KEY));
        assert!(!text.contains(SETTINGS_KEY));
    }
}

#[test]
fn chat_call_secret_safe_failure() {
    let mut settings = AnthropicSettings::default();
    settings.api_key = Some(SecretString::from(SETTINGS_KEY.to_owned()));
    assert!(!format!("{settings:?}").contains(SETTINGS_KEY));

    let mut rejected_setup = Setup::default();
    rejected_setup.acompletion = true;
    rejected_setup.trip = Trip::Status {
        status_code: 401,
        headers: failure_headers(),
        body: b"{\"error\":{\"message\":\"invalid api key\"}}".to_vec(),
        exception_headers: false,
    };
    let rejected = execute(rejected_setup);
    assert_eq!(rejected.status(), Some(401));
    assert_secret_safe(&rejected);

    // a sync failure keeps the pre_call record, which masks the credential rather than dropping the header
    let mut refused_setup = Setup::default();
    refused_setup.trip = Trip::Status {
        status_code: 401,
        headers: failure_headers(),
        body: b"{\"error\":{\"message\":\"invalid api key\"}}".to_vec(),
        exception_headers: false,
    };
    let refused = execute(refused_setup);
    assert_eq!(refused.status(), Some(401));
    assert_secret_safe(&refused);
    let masked = refused
        .detail(&["additional_args", "headers", "x-api-key"])
        .unwrap();
    assert_ne!(masked, &Json::Text(CALLER_KEY.to_owned()));

    let mut unreachable_setup = Setup::default();
    unreachable_setup.acompletion = true;
    unreachable_setup.trip = Trip::Unknown("connect failed");
    let unreachable = execute(unreachable_setup);
    assert_secret_safe(&unreachable);

    let mut unprepared_setup = Setup::default();
    unprepared_setup.transform_failure = Some("the config refused");
    let unprepared = execute(unprepared_setup);
    assert_secret_safe(&unprepared);
}

use crate::agentcore_request::payload::transform_request;
use crate::agentcore_request::policy::{
    HAS_CUSTOM_STREAM_WRAPPER, SHOULD_FAKE_STREAM, SUPPORTED_OPENAI_PARAMS,
    SUPPORTS_STREAM_PARAM_IN_BODY, get_error_class,
};
use crate::agentcore_request::tests::support::{generated_uuid, header, headers, messages, params};
use crate::bedrock_settings::{ErrorHeaders, HeaderError, HeaderValue};

pub(super) fn stream_body_policy() {
    assert_policy_flags();
    assert_environment_and_mapping();
    assert_error_factory();
}

fn policy_flags() -> (usize, bool, bool, bool) {
    (
        SUPPORTED_OPENAI_PARAMS.len(),
        HAS_CUSTOM_STREAM_WRAPPER,
        SUPPORTS_STREAM_PARAM_IN_BODY,
        SHOULD_FAKE_STREAM,
    )
}

fn assert_policy_flags() {
    assert_eq!(policy_flags(), (0, true, false, false));
}

fn assert_environment_and_mapping() {
    let supplied = params(r#"{"temperature":0.5,"stream":true,"qualifier":"prod"}"#);
    let before = supplied.clone();
    let mut request_headers = headers(&[("x-request-id", Some("abc"))]);
    transform_request(
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        &supplied,
        &params("{}"),
        &mut request_headers,
        generated_uuid(),
    )
    .unwrap();
    assert_eq!(
        header(&request_headers, "x-request-id"),
        Some("abc"),
        "environment validation leaves caller headers in place"
    );
    assert_eq!(
        supplied, before,
        "mapping leaves the optional map unchanged"
    );
}

fn assert_error_factory() {
    let headers = ErrorHeaders::Http(vec![
        ("x-request-id".to_owned(), "abc".to_owned()),
        ("retry-after".to_owned(), "3".to_owned()),
    ]);
    let error = get_error_class("upstream refused".to_owned(), 429, Some(headers)).unwrap();
    assert_eq!(error.error.status_code, 429);
    assert_eq!(error.error.response.status_code, 429);
    assert!(!error.error.status_code_is_synthesized);
    assert_eq!(
        error.response_headers,
        vec![
            ("x-request-id".to_owned(), "abc".to_owned()),
            ("retry-after".to_owned(), "3".to_owned())
        ]
    );
    // B682 validates only mapping headers; a non-ASCII name is refused, not passed on
    let non_ascii = ErrorHeaders::Mapping(vec![(
        "x-\u{e9}".to_owned(),
        HeaderValue::Text("x".to_owned()),
    )]);
    assert!(matches!(
        get_error_class("bad".to_owned(), 400, Some(non_ascii)),
        Err(HeaderError::NotAscii { name }) if name == "x-\u{e9}"
    ));
}

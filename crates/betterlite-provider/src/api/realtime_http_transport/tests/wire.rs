use std::sync::atomic::Ordering;

use super::*;

// A config naming a URL or header HTTP cannot carry: the pre-call has run, no send was reached, and the typed error names the piece
fn session_with(config: &TestConfig, source: &Source) -> (RealtimeHttpError, LoggingState) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let mut state = logging_state(&redactor);
    let envelope = logging_env(&redactor);
    let data = session_request();
    let answer = runtime().block_on(async {
        let deps = TransportDeps {
            config: Some(config),
            source,
            supplied: None,
            limits: ApiLimits::default(),
        };
        let call = session_call(SessionEndpoint::ClientSecrets, "http://127.0.0.1:9", &data);
        let mut log = LogContext {
            state: &mut state,
            env: &envelope,
        };
        session_post(&deps, &call, &mut log).await
    });
    let Err(error) = answer else {
        panic!("the wire build refuses this request");
    };
    (error, state)
}

#[test]
fn a_base_that_is_not_a_url_fails_the_wire_build() {
    let source = test_source();
    let config = TestConfig {
        fail: false,
        header: HeaderKind::Text,
        url: UrlKind::Broken,
    };
    let (error, state) = session_with(&config, &source);
    let RealtimeHttpError::RequestUrl { url, .. } = error else {
        panic!("an unusable base is the request URL error");
    };
    assert_eq!(url, "not-a-url/custom//none");
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Object(session_request()))
    );
    assert_eq!(source.created.load(Ordering::SeqCst), 1);
}

#[test]
fn a_bad_header_name_fails_the_wire_build() {
    let source = test_source();
    let config = TestConfig {
        fail: false,
        header: HeaderKind::BadName,
        url: UrlKind::Good,
    };
    let (error, state) = session_with(&config, &source);
    let RealtimeHttpError::HeaderName { name, .. } = error else {
        panic!("a header name HTTP cannot carry is the header name error");
    };
    assert_eq!(name, "bad name");
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Object(session_request()))
    );
    assert_eq!(source.created.load(Ordering::SeqCst), 1);
}

#[test]
fn a_bad_header_value_fails_the_wire_build() {
    let source = test_source();
    let config = TestConfig {
        fail: false,
        header: HeaderKind::BadValue,
        url: UrlKind::Good,
    };
    let (error, state) = session_with(&config, &source);
    let RealtimeHttpError::HeaderValue { name, .. } = error else {
        panic!("a header value HTTP cannot carry is the header value error");
    };
    assert_eq!(name, "api-key");
    assert_eq!(
        state.model_call_details.get("input"),
        Some(&Json::Object(session_request()))
    );
    assert_eq!(source.created.load(Ordering::SeqCst), 1);
}

use betterlite_config::tls::Verification;
use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use crate::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use crate::realtime::error::RealtimeError;
use crate::realtime::failure::{ClientAction, RealtimeClose, RealtimeFailure, client_action};
use crate::realtime::headers::additional_headers;
use crate::realtime::http_config::validate_environment;
use crate::realtime::ssl::{RealtimeSsl, ssl_config};
use crate::realtime::url::QueryParams;

use super::fixture::{
    DEFAULT_URL, KEY, MAX_SIZE, MODEL, RecordingLogger, http_auth_settings, http_bases,
    logged_plan, params, plan_for, prepare, redactor, url_for,
};

fn example_url(query_params: &QueryParams) -> String {
    url_for(Some("https://example.test/"), Some(query_params))
}

fn forced_model(query_params: &QueryParams) -> Option<String> {
    plan_for(
        Some("https://example.test/"),
        Some(query_params),
        &Verification::Enabled,
    )
    .force_transcription_model
}

pub(super) fn ws_wss_scheme() {
    let (default_plan, logger) = logged_plan(None, None, false);
    assert_eq!(default_plan.url, DEFAULT_URL);
    assert_eq!(default_plan.ssl, RealtimeSsl::Shared);
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(logger.calls[0].api_base, default_plan.url);

    assert_eq!(url_for(Some("https://api.openai.com/"), None), DEFAULT_URL);

    let plain = plan_for(
        Some("http://127.0.0.1:8080/base/path"),
        None,
        &Verification::Enabled,
    );
    assert_eq!(
        plain.url,
        "ws://127.0.0.1:8080/v1/realtime?model=realtime-a"
    );
    assert_eq!(plain.ssl, RealtimeSsl::None);

    assert_eq!(
        url_for(
            Some("https://example.test/a/b?q=1#frag"),
            Some(&params(&[]))
        ),
        "wss://example.test/v1/realtime?q=1#frag"
    );

    let empty_base = prepare(
        Some(""),
        None,
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    );
    assert!(matches!(
        empty_base,
        Err(RealtimeError::InvalidApiBase { .. })
    ));
}

pub(super) fn replace_path_with_v1_realtime() {
    assert_eq!(
        url_for(Some("https://api.openai.com/v1/realtime"), None),
        DEFAULT_URL
    );
    assert_eq!(url_for(Some("https://api.openai.com"), None), DEFAULT_URL);

    let empty_query = params(&[]);
    assert_eq!(
        url_for(Some("https://example.test/a/b?q=1"), Some(&empty_query)),
        "wss://example.test/v1/realtime?q=1"
    );
    assert_eq!(
        url_for(Some("https://example.test//a//b"), Some(&empty_query)),
        "wss://example.test/v1/realtime"
    );

    let chosen = params(&[("model", Some("m"))]);
    assert_eq!(
        url_for(Some("https://example.test/a/b?q=1"), Some(&chosen)),
        "wss://example.test/v1/realtime?model=m"
    );
    assert_eq!(
        url_for(Some("https://example.test/a/b?q=1#frag"), Some(&chosen)),
        "wss://example.test/v1/realtime?model=m#frag"
    );

    assert_eq!(
        url_for(Some("https://example.test:443/x"), None),
        "wss://example.test/v1/realtime?model=realtime-a"
    );
    assert_eq!(
        url_for(Some("wss://example.test/x"), None),
        "wss://example.test/v1/realtime?model=realtime-a"
    );
}

pub(super) fn query_model_or_explicit() {
    fallback_and_suppression();
    transcription_intent();
    escaped_and_null_query();
}

fn fallback_and_suppression() {
    let (fallback_plan, fallback_log) = logged_plan(None, None, false);
    assert_eq!(
        fallback_log.calls[0].query_params,
        params(&[("model", Some(MODEL))])
    );
    assert_eq!(fallback_plan.force_transcription_model, None);

    let (suppressed, empty_log) =
        logged_plan(Some("https://example.test/"), Some(&params(&[])), false);
    assert_eq!(suppressed.url, "wss://example.test/v1/realtime");
    assert!(empty_log.calls[0].query_params.is_empty());
}

fn transcription_intent() {
    let transcription = params(&[
        ("model", Some("explicit")),
        ("intent", Some("transcription")),
    ]);
    assert_eq!(
        example_url(&transcription),
        "wss://example.test/v1/realtime?model=explicit&intent=transcription"
    );
    assert_eq!(forced_model(&transcription).as_deref(), Some(MODEL));

    let chat = params(&[("model", Some("explicit")), ("intent", Some("chat"))]);
    assert_eq!(forced_model(&chat), None);
}

fn escaped_and_null_query() {
    let starred = params(&[("model", Some("a*b~c"))]);
    assert_eq!(
        example_url(&starred),
        "wss://example.test/v1/realtime?model=a%2Ab~c"
    );

    let null_intent = params(&[("intent", None), ("model", Some("m"))]);
    assert_eq!(
        example_url(&null_intent),
        "wss://example.test/v1/realtime?intent=&model=m"
    );
    assert_eq!(forced_model(&null_intent), None);
}

pub(super) fn beta_header_only_when_client_sends() {
    let (plain, logger) = logged_plan(Some("https://example.test/"), None, false);
    assert_eq!(plain.headers.len(), 1);
    assert_eq!(
        plain.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(plain.headers.get("OpenAI-Beta"), None);
    assert_eq!(logger.calls[0].headers, plain.headers);
    assert!(plain.event_normalizer.is_none());

    let (beta, beta_log) = logged_plan(Some("https://example.test/"), None, true);
    assert_eq!(
        beta.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(
        beta.headers.get("OpenAI-Beta").map(String::as_str),
        Some("realtime=v1")
    );
    assert!(beta.event_normalizer.is_none());
    assert_eq!(beta_log.calls.len(), 1);

    let key = SecretString::from(String::new());
    let headers = additional_headers(&key, false);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
}

pub(super) fn http_session_paths() {
    default_http_paths();
    explicit_and_global_bases();
}

fn default_http_paths() {
    let empty = http_bases(None, None);
    assert_eq!(empty.api_base(None), "https://api.openai.com");
    assert_eq!(
        empty.complete_url(None),
        "https://api.openai.com/v1/realtime/client_secrets"
    );
    assert_eq!(
        empty.calls_url(None),
        "https://api.openai.com/v1/realtime/calls"
    );
    assert_eq!(
        empty.transcription_session_url(None),
        "https://api.openai.com/v1/realtime/transcription_sessions"
    );
}

fn explicit_and_global_bases() {
    let empty = http_bases(None, None);
    let explicit = "http://explicit.example/v1/";
    assert_eq!(empty.api_base(Some(explicit)), explicit);
    assert_eq!(
        empty.complete_url(Some(explicit)),
        "http://explicit.example/v1/realtime/client_secrets"
    );
    assert_eq!(
        empty.calls_url(Some("http://explicit.example///")),
        "http://explicit.example/v1/realtime/calls"
    );

    let doubled = http_bases(Some("http://x/v1/v1"), None);
    assert_eq!(
        doubled.complete_url(None),
        "http://x/v1/v1/realtime/client_secrets"
    );

    let globals = http_bases(Some("http://global.example"), Some("http://env.example/"));
    assert_eq!(globals.api_base(None), "http://global.example");
    assert_eq!(globals.api_base(Some("")), "http://global.example");
    assert_eq!(
        globals.transcription_session_url(None),
        "http://global.example/v1/realtime/transcription_sessions"
    );

    let env_only = http_bases(Some(""), Some("http://env.example/"));
    assert_eq!(env_only.api_base(None), "http://env.example/");
    assert_eq!(
        env_only.calls_url(None),
        "http://env.example/v1/realtime/calls"
    );
}

pub(super) fn missing_api_key() {
    let mut logger = RecordingLogger::default();
    let config = RealtimeConnectSettings {
        api_base: None,
        api_key: None,
        model: MODEL,
        query_params: None,
        beta_realtime: false,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: MAX_SIZE,
    };
    let error = prepare_connection(&config, &mut logger)
        .err()
        .expect("a missing key is refused");
    assert!(matches!(error, RealtimeError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "api_key is required for OpenAI realtime calls"
    );
    assert!(logger.calls.is_empty());

    let blank_key = SecretString::from(String::new());
    let keyed_settings = RealtimeConnectSettings {
        api_base: None,
        api_key: Some(&blank_key),
        model: MODEL,
        query_params: None,
        beta_realtime: false,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: MAX_SIZE,
    };
    let plan = prepare_connection(&keyed_settings, &mut logger).unwrap();
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
    assert_eq!(logger.calls.len(), 1);
}

pub(super) fn handshake_refusal() {
    let redactor = redactor();
    for status_code in [401, 403, 429, 500] {
        let action = client_action(RealtimeFailure::Handshake { status_code }, &redactor).unwrap();
        assert_eq!(action, ClientAction::RefuseHandshake { status_code });
    }
}

pub(super) fn redacted_close_reason() {
    redacted_other_errors();
    ignored_close_failures();
    unknown_close_failure();
}

fn redacted_other_errors() {
    let redactor = redactor();
    assert_eq!(
        client_action(
            RealtimeFailure::Other {
                message: "connect failed with Bearer abcdefghij1234567890",
            },
            &redactor,
        )
        .unwrap(),
        ClientAction::Close(RealtimeClose {
            code: 1011,
            reason: String::from("Internal server error: connect failed with REDACTED"),
        })
    );
    assert_eq!(
        client_action(
            RealtimeFailure::Other {
                message: "connection reset",
            },
            &redactor,
        )
        .unwrap(),
        ClientAction::Close(RealtimeClose {
            code: 1011,
            reason: String::from("Internal server error: connection reset"),
        })
    );
}

fn ignored_close_failures() {
    let redactor = redactor();
    assert_eq!(
        client_action(
            RealtimeFailure::CloseFailed {
                message: "connection reset",
                close_error: "sent 1011 (internal error) already completed",
            },
            &redactor,
        )
        .unwrap(),
        ClientAction::IgnoreCloseFailure {
            attempted: RealtimeClose {
                code: 1011,
                reason: String::from("Internal server error: connection reset"),
            },
        }
    );
    let library = client_action(
        RealtimeFailure::CloseFailed {
            message: "connection reset",
            close_error: "websocket.close() is not allowed here",
        },
        &redactor,
    )
    .unwrap();
    assert!(matches!(library, ClientAction::IgnoreCloseFailure { .. }));
}

fn unknown_close_failure() {
    let redactor = redactor();
    let failure = client_action(
        RealtimeFailure::CloseFailed {
            message: "connection reset",
            close_error: "socket is gone",
        },
        &redactor,
    )
    .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Unexpected error while closing WebSocket: socket is gone"
    );
}

pub(super) fn realtime_ssl_and_size() {
    let plain_url = "ws://example.test/v1/realtime";
    let secure_url = "wss://example.test/v1/realtime";
    let ca_file = Verification::CaFile(String::from("/etc/ca.pem"));
    assert_eq!(
        ssl_config(plain_url, &Verification::Enabled),
        RealtimeSsl::None
    );
    assert_eq!(
        ssl_config(plain_url, &Verification::Disabled),
        RealtimeSsl::None
    );
    assert_eq!(ssl_config(plain_url, &ca_file), RealtimeSsl::None);
    assert_eq!(
        ssl_config(secure_url, &Verification::Enabled),
        RealtimeSsl::Shared
    );
    assert_eq!(
        ssl_config(secure_url, &Verification::Disabled),
        RealtimeSsl::Insecure
    );
    assert_eq!(ssl_config(secure_url, &ca_file), RealtimeSsl::Shared);

    let plain_plan = plan_for(Some("http://example.test"), None, &Verification::Disabled);
    assert_eq!(plain_plan.ssl, RealtimeSsl::None);
    assert_eq!(plain_plan.max_message_size_bytes, MAX_SIZE);
    let tls_plan = plan_for(Some("https://example.test"), None, &Verification::Disabled);
    assert_eq!(tls_plan.ssl, RealtimeSsl::Insecure);

    let key = SecretString::from(KEY.to_owned());
    let settings = RealtimeConnectSettings {
        api_base: Some("https://example.test"),
        api_key: Some(&key),
        model: MODEL,
        query_params: None,
        beta_realtime: false,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: 8_192,
    };
    let sized = prepare_connection(&settings, &mut RecordingLogger::default()).unwrap();
    assert_eq!(sized.max_message_size_bytes, 8_192);
}

pub(super) fn realtime_http_auth() {
    api_key_precedence();
    validated_headers();
}

fn api_key_precedence() {
    let all = http_auth_settings(Some("sk-litellm"), Some("sk-openai"), Some("sk-env"));
    assert_eq!(
        all.api_key(Some("sk-explicit")).expose_secret(),
        "sk-explicit"
    );
    assert_eq!(all.api_key(None).expose_secret(), "sk-litellm");
    assert_eq!(all.api_key(Some("")).expose_secret(), "sk-litellm");

    let openai_only = http_auth_settings(None, Some("sk-openai"), Some("sk-env"));
    assert_eq!(openai_only.api_key(None).expose_secret(), "sk-openai");
    let env_only = http_auth_settings(None, None, Some("sk-env"));
    assert_eq!(env_only.api_key(None).expose_secret(), "sk-env");
    let nothing = http_auth_settings(None, None, None);
    assert_eq!(nothing.api_key(None).expose_secret(), "");
}

fn validated_headers() {
    let headers = Headers::from([
        (String::from("Authorization"), String::from("Bearer old")),
        (String::from("Content-Type"), String::from("text/plain")),
        (String::from("X-Test"), String::from("kept")),
    ]);
    let validated = validate_environment(&headers, Some(&SecretString::from("sk-valid")));
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer sk-valid")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(validated.get("X-Test").map(String::as_str), Some("kept"));
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer old")
    );

    let empty = validate_environment(&Headers::new(), None);
    assert_eq!(
        empty.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
    assert_eq!(
        empty.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

use betterlite_config::tls::Verification;
use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::Headers;
use crate::realtime::connect::{RealtimeConnectSettings, prepare_connection};
use crate::realtime::error::RealtimeError;
use crate::realtime::failure::{ClientAction, RealtimeClose, RealtimeFailure, client_action};
use crate::realtime::headers::additional_headers;
use crate::realtime::http_config::{RealtimeHttpSettings, validate_environment};
use crate::realtime::ssl::{RealtimeSsl, ssl_config};

use super::fixture::{KEY, MAX_SIZE, MODEL, RecordingLogger, params, prepare, redactor};

pub(super) fn ws_wss_scheme() {
    let mut logger = RecordingLogger::default();
    let default_plan = prepare(None, None, false, &Verification::Enabled, &mut logger).unwrap();
    assert_eq!(
        default_plan.url,
        "wss://api.openai.com/v1/realtime?model=realtime-a"
    );
    assert_eq!(default_plan.ssl, RealtimeSsl::Shared);
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(logger.calls[0].api_base, default_plan.url);

    let secure = prepare(
        Some("https://api.openai.com/"),
        None,
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        secure.url,
        "wss://api.openai.com/v1/realtime?model=realtime-a"
    );

    let plain = prepare(
        Some("http://127.0.0.1:8080/base/path"),
        None,
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        plain.url,
        "ws://127.0.0.1:8080/v1/realtime?model=realtime-a"
    );
    assert_eq!(plain.ssl, RealtimeSsl::None);

    let empty_query = params(&[]);
    let kept = prepare(
        Some("https://example.test/a/b?q=1#frag"),
        Some(&empty_query),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(kept.url, "wss://example.test/v1/realtime?q=1#frag");

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
    let replaced = prepare(
        Some("https://api.openai.com/v1/realtime"),
        None,
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        replaced.url,
        "wss://api.openai.com/v1/realtime?model=realtime-a"
    );

    let no_slash = prepare(
        Some("https://api.openai.com"),
        None,
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        no_slash.url,
        "wss://api.openai.com/v1/realtime?model=realtime-a"
    );

    let empty_query = params(&[]);
    let with_query = prepare(
        Some("https://example.test/a/b?q=1"),
        Some(&empty_query),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(with_query.url, "wss://example.test/v1/realtime?q=1");

    let chosen = params(&[("model", Some("m"))]);
    let replaced_query = prepare(
        Some("https://example.test/a/b?q=1"),
        Some(&chosen),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(replaced_query.url, "wss://example.test/v1/realtime?model=m");

    let fragment = prepare(
        Some("https://example.test/a/b?q=1#frag"),
        Some(&chosen),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(fragment.url, "wss://example.test/v1/realtime?model=m#frag");

    let doubled = prepare(
        Some("https://example.test//a//b"),
        Some(&empty_query),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(doubled.url, "wss://example.test/v1/realtime");

    let default_port = prepare(
        Some("https://example.test:443/x"),
        None,
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        default_port.url,
        "wss://example.test/v1/realtime?model=realtime-a"
    );

    let already_ws = prepare(
        Some("wss://example.test/x"),
        None,
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        already_ws.url,
        "wss://example.test/v1/realtime?model=realtime-a"
    );
}

pub(super) fn query_model_or_explicit() {
    let mut logger = RecordingLogger::default();
    let fallback = prepare(
        Some("https://example.test/"),
        None,
        false,
        &Verification::Enabled,
        &mut logger,
    )
    .unwrap();
    assert_eq!(
        fallback.url,
        "wss://example.test/v1/realtime?model=realtime-a"
    );
    assert_eq!(
        logger.calls[0].query_params,
        params(&[("model", Some("realtime-a"))])
    );
    assert_eq!(fallback.force_transcription_model, None);

    let empty = params(&[]);
    let mut empty_logger = RecordingLogger::default();
    let suppressed = prepare(
        Some("https://example.test/"),
        Some(&empty),
        false,
        &Verification::Enabled,
        &mut empty_logger,
    )
    .unwrap();
    assert_eq!(suppressed.url, "wss://example.test/v1/realtime");
    assert!(empty_logger.calls[0].query_params.is_empty());

    let transcription = params(&[
        ("model", Some("explicit")),
        ("intent", Some("transcription")),
    ]);
    let forced = prepare(
        Some("https://example.test/"),
        Some(&transcription),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        forced.url,
        "wss://example.test/v1/realtime?model=explicit&intent=transcription"
    );
    assert_eq!(
        forced.force_transcription_model.as_deref(),
        Some("realtime-a")
    );

    let chat = params(&[("model", Some("explicit")), ("intent", Some("chat"))]);
    let unchanged = prepare(
        Some("https://example.test/"),
        Some(&chat),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(unchanged.force_transcription_model, None);

    let starred = params(&[("model", Some("a*b~c"))]);
    let starred_plan = prepare(
        Some("https://example.test/"),
        Some(&starred),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        starred_plan.url,
        "wss://example.test/v1/realtime?model=a%2Ab~c"
    );

    let null_intent = params(&[("intent", None), ("model", Some("m"))]);
    let null_plan = prepare(
        Some("https://example.test/"),
        Some(&null_intent),
        false,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        null_plan.url,
        "wss://example.test/v1/realtime?intent=&model=m"
    );
    assert_eq!(null_plan.force_transcription_model, None);
}

pub(super) fn beta_header_only_when_client_sends() {
    let mut logger = RecordingLogger::default();
    let plain = prepare(
        Some("https://example.test/"),
        None,
        false,
        &Verification::Enabled,
        &mut logger,
    )
    .unwrap();
    assert_eq!(plain.headers.len(), 1);
    assert_eq!(
        plain.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(plain.headers.get("OpenAI-Beta"), None);
    assert_eq!(logger.calls[0].headers, plain.headers);
    assert!(plain.event_normalizer.is_none());

    let beta = prepare(
        Some("https://example.test/"),
        None,
        true,
        &Verification::Enabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(
        beta.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(
        beta.headers.get("OpenAI-Beta").map(String::as_str),
        Some("realtime=v1")
    );
    assert!(beta.event_normalizer.is_none());

    let key = SecretString::from(String::new());
    let headers = additional_headers(&key, false);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer ")
    );
}

pub(super) fn http_session_paths() {
    let empty = RealtimeHttpSettings {
        litellm_api_base: None,
        litellm_api_key: None,
        litellm_openai_key: None,
        openai_api_base: None,
        openai_api_key: None,
    };
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

    assert_eq!(
        empty.api_base(Some("http://explicit.example/v1/")),
        "http://explicit.example/v1/"
    );
    assert_eq!(
        empty.complete_url(Some("http://explicit.example/v1/")),
        "http://explicit.example/v1/realtime/client_secrets"
    );
    assert_eq!(
        empty.calls_url(Some("http://explicit.example///")),
        "http://explicit.example/v1/realtime/calls"
    );
    let doubled = RealtimeHttpSettings {
        litellm_api_base: Some(String::from("http://x/v1/v1")),
        litellm_api_key: None,
        litellm_openai_key: None,
        openai_api_base: None,
        openai_api_key: None,
    };
    assert_eq!(
        doubled.complete_url(None),
        "http://x/v1/v1/realtime/client_secrets"
    );

    let globals = RealtimeHttpSettings {
        litellm_api_base: Some(String::from("http://global.example")),
        litellm_api_key: None,
        litellm_openai_key: None,
        openai_api_base: Some(String::from("http://env.example/")),
        openai_api_key: None,
    };
    assert_eq!(globals.api_base(None), "http://global.example");
    assert_eq!(globals.api_base(Some("")), "http://global.example");
    assert_eq!(
        globals.transcription_session_url(None),
        "http://global.example/v1/realtime/transcription_sessions"
    );

    let env_only = RealtimeHttpSettings {
        litellm_api_base: Some(String::new()),
        litellm_api_key: None,
        litellm_openai_key: None,
        openai_api_base: Some(String::from("http://env.example/")),
        openai_api_key: None,
    };
    assert_eq!(env_only.api_base(None), "http://env.example/");
    assert_eq!(
        env_only.calls_url(None),
        "http://env.example/v1/realtime/calls"
    );
}

pub(super) fn missing_api_key() {
    let mut logger = RecordingLogger::default();
    let settings = RealtimeConnectSettings {
        api_base: None,
        api_key: None,
        model: MODEL,
        query_params: None,
        beta_realtime: false,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: MAX_SIZE,
    };
    let error = prepare_connection(&settings, &mut logger)
        .err()
        .expect("a missing key is refused");
    assert!(matches!(error, RealtimeError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "api_key is required for OpenAI realtime calls"
    );
    assert!(logger.calls.is_empty());

    let empty_key = SecretString::from(String::new());
    let settings = RealtimeConnectSettings {
        api_base: None,
        api_key: Some(&empty_key),
        model: MODEL,
        query_params: None,
        beta_realtime: false,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes: MAX_SIZE,
    };
    let plan = prepare_connection(&settings, &mut logger).unwrap();
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
    let redactor = redactor();
    let other = client_action(
        RealtimeFailure::Other {
            message: "connect failed with Bearer abcdefghij1234567890",
        },
        &redactor,
    )
    .unwrap();
    assert_eq!(
        other,
        ClientAction::Close(RealtimeClose {
            code: 1011,
            reason: String::from("Internal server error: connect failed with REDACTED"),
        })
    );

    let plain = client_action(
        RealtimeFailure::Other {
            message: "connection reset",
        },
        &redactor,
    )
    .unwrap();
    assert_eq!(
        plain,
        ClientAction::Close(RealtimeClose {
            code: 1011,
            reason: String::from("Internal server error: connection reset"),
        })
    );

    let completed = client_action(
        RealtimeFailure::CloseFailed {
            message: "connection reset",
            close_error: "sent 1011 (internal error) already completed",
        },
        &redactor,
    )
    .unwrap();
    assert_eq!(
        completed,
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
    let ws_url = "ws://example.test/v1/realtime";
    let ca_file = Verification::CaFile(String::from("/etc/ca.pem"));
    assert_eq!(
        ssl_config(ws_url, &Verification::Enabled),
        RealtimeSsl::None
    );
    assert_eq!(
        ssl_config(ws_url, &Verification::Disabled),
        RealtimeSsl::None
    );
    assert_eq!(ssl_config(ws_url, &ca_file), RealtimeSsl::None);

    let wss_url = "wss://example.test/v1/realtime";
    assert_eq!(
        ssl_config(wss_url, &Verification::Enabled),
        RealtimeSsl::Shared
    );
    assert_eq!(
        ssl_config(wss_url, &Verification::Disabled),
        RealtimeSsl::Insecure
    );
    assert_eq!(ssl_config(wss_url, &ca_file), RealtimeSsl::Shared);

    let http_plan = prepare(
        Some("http://example.test"),
        None,
        false,
        &Verification::Disabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(http_plan.ssl, RealtimeSsl::None);
    assert_eq!(http_plan.max_message_size_bytes, MAX_SIZE);

    let https_plan = prepare(
        Some("https://example.test"),
        None,
        false,
        &Verification::Disabled,
        &mut RecordingLogger::default(),
    )
    .unwrap();
    assert_eq!(https_plan.ssl, RealtimeSsl::Insecure);

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
    let litellm_key = SecretString::from("sk-litellm".to_owned());
    let openai_key = SecretString::from("sk-openai".to_owned());
    let env_key = SecretString::from("sk-env".to_owned());
    let all = RealtimeHttpSettings {
        litellm_api_base: None,
        litellm_api_key: Some(litellm_key),
        litellm_openai_key: Some(openai_key),
        openai_api_base: None,
        openai_api_key: Some(env_key),
    };
    assert_eq!(
        all.api_key(Some("sk-explicit")).expose_secret(),
        "sk-explicit"
    );
    assert_eq!(all.api_key(None).expose_secret(), "sk-litellm");
    assert_eq!(all.api_key(Some("")).expose_secret(), "sk-litellm");

    let openai_only = RealtimeHttpSettings {
        litellm_api_base: None,
        litellm_api_key: None,
        litellm_openai_key: all.litellm_openai_key.clone(),
        openai_api_base: None,
        openai_api_key: all.openai_api_key.clone(),
    };
    assert_eq!(openai_only.api_key(None).expose_secret(), "sk-openai");

    let env_only = RealtimeHttpSettings {
        litellm_api_base: None,
        litellm_api_key: None,
        litellm_openai_key: None,
        openai_api_base: None,
        openai_api_key: all.openai_api_key.clone(),
    };
    assert_eq!(env_only.api_key(None).expose_secret(), "sk-env");

    let nothing = RealtimeHttpSettings {
        litellm_api_base: None,
        litellm_api_key: None,
        litellm_openai_key: None,
        openai_api_base: None,
        openai_api_key: None,
    };
    assert_eq!(nothing.api_key(None).expose_secret(), "");

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

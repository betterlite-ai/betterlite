use betterlite_config::tls::Verification;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::realtime::failure::RealtimeClose;
use secrecy::{ExposeSecret as _, SecretString};

use crate::realtime::error::RealtimeError;
use crate::realtime::failure::{
    ClientAction, RealtimeFailure, SESSION_CLOSE_CODE, SESSION_ERROR_MESSAGE, SESSION_ERROR_TYPE,
    client_action,
};
use crate::realtime::http_config::{
    DEFAULT_API_VERSION, RealtimeHttpSettings, realtime_calls_headers, validate_environment,
};
use crate::realtime::protocol::{BETA_PROTOCOL, GA_PROTOCOL, client_protocol, uses_beta_protocol};
use crate::realtime::ssl::{RealtimeSsl, ssl_config};

use super::fixture::{
    API_VERSION, KEY, MAX_SIZE, MODEL, TOKEN, connection, headers, http_settings, params,
    with_version,
};

pub(super) fn beta_realtime_deployment_query() {
    beta_plan();
    beta_intent();
    missing_inputs();
}

fn beta_plan() {
    let plain = connection(Some("https://example.test"), Some(API_VERSION), None, None);
    let plan = plain.plan();
    assert_eq!(
        plan.url,
        "wss://example.test/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a"
    );
    assert_eq!(
        plan.headers,
        Headers::from([(String::from("api-key"), String::from(KEY))])
    );
    assert!(plan.backend_uses_beta_protocol);
    assert_eq!(plan.force_transcription_model, None);
    assert!(plan.event_normalizer.is_none());
}

fn beta_intent() {
    let transcription = params(&[("intent", Some("transcription"))]);
    let transcribed = connection(
        Some("https://example.test/region/"),
        Some(API_VERSION),
        Some("beta"),
        Some(&transcription),
    );
    assert_eq!(
        transcribed.url(),
        "wss://example.test/region//openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a&intent=transcription"
    );
    assert_eq!(
        transcribed.plan().force_transcription_model.as_deref(),
        Some(MODEL)
    );

    let null_intent = params(&[("intent", None)]);
    let nulled = connection(
        Some("https://example.test"),
        Some(API_VERSION),
        None,
        Some(&null_intent),
    );
    assert_eq!(
        nulled.url(),
        "wss://example.test/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a"
    );

    let empty_intent = params(&[("intent", Some(""))]);
    let empty = connection(
        Some("https://example.test"),
        Some(API_VERSION),
        None,
        Some(&empty_intent),
    );
    assert_eq!(
        empty.url(),
        "wss://example.test/openai/realtime?api-version=2024-10-01-preview&deployment=realtime-a"
    );
}

fn missing_inputs() {
    let no_base = connection(None, Some(API_VERSION), None, None);
    assert_eq!(no_base.error(), RealtimeError::MissingApiBase);
    assert_eq!(
        no_base.error().to_string(),
        "api_base is required for Azure OpenAI calls"
    );

    let no_version = connection(Some("https://example.test"), None, None, None);
    assert_eq!(no_version.error(), RealtimeError::MissingApiVersion);
    assert_eq!(
        no_version.error().to_string(),
        "api_version is required for Azure OpenAI calls"
    );

    let ga_without_version = connection(Some("https://example.test"), None, Some("GA"), None);
    assert_eq!(
        ga_without_version.url(),
        "wss://example.test/openai/v1/realtime?model=realtime-a"
    );

    let both_missing = connection(None, None, None, None);
    assert_eq!(both_missing.error(), RealtimeError::MissingApiBase);
}

pub(super) fn ga_realtime_url() {
    schemes_and_paths();
    model_insertion();
    intent_append();
}

fn schemes_and_paths() {
    let secure = connection(Some("https://example.test"), None, Some("GA"), None);
    assert_eq!(
        secure.url(),
        "wss://example.test/openai/v1/realtime?model=realtime-a"
    );
    assert!(!secure.plan().backend_uses_beta_protocol);

    let empty = params(&[]);
    let plain = connection(
        Some("http://example.test/base/x"),
        Some("ignored"),
        Some("v1"),
        Some(&empty),
    );
    assert_eq!(plain.url(), "http://example.test/base/x/openai/v1/realtime");

    let trailing = connection(Some("https://example.test/"), None, Some("V1"), None);
    assert_eq!(
        trailing.url(),
        "wss://example.test//openai/v1/realtime?model=realtime-a"
    );

    let existing = connection(Some("https://example.test/a/b?q=1"), None, Some("ga"), None);
    assert_eq!(
        existing.url(),
        "wss://example.test/a/b?q=1/openai/v1/realtime?model=realtime-a"
    );
}

fn model_insertion() {
    let chosen = params(&[("model", Some("customer")), ("foo", Some("bar"))]);
    let explicit = connection(
        Some("https://example.test"),
        None,
        Some("GA"),
        Some(&chosen),
    );
    assert_eq!(
        explicit.url(),
        "wss://example.test/openai/v1/realtime?model=realtime-a"
    );

    let null_model = params(&[("model", None)]);
    let nulled = connection(
        Some("https://example.test"),
        None,
        Some("GA"),
        Some(&null_model),
    );
    assert_eq!(
        nulled.url(),
        "wss://example.test/openai/v1/realtime?model=realtime-a"
    );

    let transcription = params(&[("intent", Some("transcription")), ("model", Some("x"))]);
    let forced = connection(
        Some("https://example.test"),
        None,
        Some("GA"),
        Some(&transcription),
    );
    assert_eq!(
        forced.url(),
        "wss://example.test/openai/v1/realtime?intent=transcription"
    );
    assert_eq!(
        forced.plan().force_transcription_model.as_deref(),
        Some(MODEL)
    );
}

fn intent_append() {
    let chat = params(&[("model", Some("x")), ("intent", Some("a b"))]);
    let appended = connection(Some("https://example.test"), None, Some("GA"), Some(&chat));
    assert_eq!(
        appended.url(),
        "wss://example.test/openai/v1/realtime?model=realtime-a&intent=a+b"
    );

    let starred = params(&[("intent", Some("x*y~z"))]);
    let escaped = connection(
        Some("https://example.test"),
        None,
        Some("GA"),
        Some(&starred),
    );
    assert_eq!(
        escaped.url(),
        "wss://example.test/openai/v1/realtime?intent=x%2Ay~z"
    );

    let null_intent = params(&[("intent", None)]);
    let quiet = connection(
        Some("https://example.test"),
        None,
        Some("GA"),
        Some(&null_intent),
    );
    assert_eq!(quiet.url(), "wss://example.test/openai/v1/realtime");
}

pub(super) fn api_version_query() {
    versions_in_urls();
    version_required_for_beta();
}

fn versions_in_urls() {
    let preview = connection(
        Some("https://example.test"),
        Some("2025-01-01-preview"),
        None,
        None,
    );
    assert_eq!(
        preview.url(),
        "wss://example.test/openai/realtime?api-version=2025-01-01-preview&deployment=realtime-a"
    );

    let spaced = connection(
        Some("https://example.test"),
        Some("2025-01-01 preview+x"),
        Some("beta"),
        None,
    );
    assert_eq!(
        spaced.url(),
        "wss://example.test/openai/realtime?api-version=2025-01-01+preview%2Bx&deployment=realtime-a"
    );

    let ga = connection(
        Some("https://example.test"),
        Some("2025-01-01"),
        Some("GA"),
        None,
    );
    assert_eq!(
        ga.url(),
        "wss://example.test/openai/v1/realtime?model=realtime-a"
    );

    let empty = connection(Some("https://example.test"), Some(""), None, None);
    assert_eq!(
        empty.url(),
        "wss://example.test/openai/realtime?api-version=&deployment=realtime-a"
    );
}

fn version_required_for_beta() {
    let beta = connection(Some("https://example.test"), None, None, None);
    assert_eq!(beta.error(), RealtimeError::MissingApiVersion);

    let named = connection(Some("https://example.test"), None, Some("beta"), None);
    assert_eq!(named.error(), RealtimeError::MissingApiVersion);

    let ga = connection(Some("https://example.test"), None, Some("V1"), None);
    assert_eq!(
        ga.url(),
        "wss://example.test/openai/v1/realtime?model=realtime-a"
    );
}

pub(super) fn api_key_versus_token() {
    key_wins();
    token_fallback();
    missing_credentials();
    refusal_ordering();
}

fn key_wins() {
    let both = connection(Some("https://example.test"), None, Some("GA"), None)
        .with_token(Some(TOKEN))
        .with_key(Some("key-four"));
    let plan = both.plan();
    assert_eq!(
        plan.headers,
        Headers::from([(String::from("api-key"), String::from("key-four"))])
    );
    assert_eq!(plan.headers.get("Authorization"), None);
}

fn token_fallback() {
    let empty_key = connection(Some("https://example.test"), None, Some("GA"), None)
        .with_key(Some(""))
        .with_token(Some(TOKEN));
    assert_eq!(
        empty_key.plan().headers,
        Headers::from([(String::from("Authorization"), format!("Bearer {TOKEN}"))])
    );

    let no_key = connection(Some("https://example.test"), None, Some("GA"), None)
        .with_key(None)
        .with_token(Some(TOKEN));
    assert_eq!(
        no_key.plan().headers,
        Headers::from([(String::from("Authorization"), format!("Bearer {TOKEN}"))])
    );
}

fn missing_credentials() {
    let bare = connection(Some("https://example.test"), None, Some("GA"), None).with_key(None);
    assert_eq!(bare.error(), RealtimeError::MissingCredentials);
    assert_eq!(
        bare.error().to_string(),
        "Missing Azure credentials for the realtime endpoint. Set an api_key, or configure Azure AD auth (azure_ad_token, tenant_id/client_id/client_secret, or a managed identity)"
    );

    let blank = connection(Some("https://example.test"), None, Some("GA"), None)
        .with_key(Some(""))
        .with_token(Some(""));
    assert_eq!(blank.error(), RealtimeError::MissingCredentials);
}

fn refusal_ordering() {
    let missing_base = connection(None, None, None, None)
        .with_key(None)
        .with_token(None);
    assert_eq!(missing_base.error(), RealtimeError::MissingApiBase);

    let missing_version = connection(Some("https://example.test"), None, None, None)
        .with_key(None)
        .with_token(None);
    assert_eq!(missing_version.error(), RealtimeError::MissingApiVersion);
}

pub(super) fn client_beta_protocol() {
    configured_wins();
    header_and_intent_select();
    construction_beta();
}

fn configured_wins() {
    let transcription = params(&[("intent", Some("transcription"))]);
    assert_eq!(client_protocol(Some("v2"), None, false), "v2");
    assert_eq!(client_protocol(Some("beta"), None, true), "beta");
    assert_eq!(client_protocol(Some(" "), None, false), " ");
    assert_eq!(
        client_protocol(Some("GA"), Some(&transcription), true),
        GA_PROTOCOL
    );
    assert_eq!(client_protocol(Some(""), None, true), BETA_PROTOCOL);
}

fn header_and_intent_select() {
    assert_eq!(client_protocol(None, None, true), BETA_PROTOCOL);
    assert_eq!(client_protocol(None, None, false), GA_PROTOCOL);

    let transcription = params(&[("intent", Some("transcription"))]);
    assert_eq!(
        client_protocol(None, Some(&transcription), true),
        GA_PROTOCOL
    );

    let chat = params(&[("intent", Some("chat"))]);
    assert_eq!(client_protocol(None, Some(&chat), true), BETA_PROTOCOL);
    assert_eq!(client_protocol(None, Some(&chat), false), GA_PROTOCOL);
}

fn construction_beta() {
    assert!(uses_beta_protocol(None));
    for protocol in ["GA", "ga", "V1", "v1"] {
        assert!(!uses_beta_protocol(Some(protocol)));
    }
    for protocol in ["beta", "v2", "", " ", "G A"] {
        assert!(uses_beta_protocol(Some(protocol)));
    }
}

pub(super) fn calls_and_transcription_headers() {
    session_paths();
    validated_headers();
    call_headers();
    ssl_forwarding();
}

fn session_paths() {
    let empty = http_settings(None, None, None, None);
    assert_eq!(
        empty.complete_url(None, None),
        "/openai/realtime/client_secrets?api-version=2024-12-17"
    );
    assert_eq!(
        empty.calls_url(None, None),
        "/openai/realtime/calls?api-version=2024-12-17"
    );
    assert_eq!(
        empty.transcription_session_url(None, None),
        "/openai/realtime/transcription_sessions?api-version=2024-12-17"
    );

    assert_eq!(
        empty.complete_url(Some("https://explicit.test/v1/"), Some("2025-04-01")),
        "https://explicit.test/v1/openai/realtime/client_secrets?api-version=2025-04-01"
    );
    assert_eq!(
        empty.calls_url(Some("https://explicit.test///"), None),
        "https://explicit.test/openai/realtime/calls?api-version=2024-12-17"
    );
}

fn validated_headers() {
    let original = headers(&[
        ("api-key", "stale"),
        ("Content-Type", "text/plain"),
        ("Authorization", "Bearer old"),
        ("X-Test", "kept"),
    ]);
    let validated = validate_environment(&original, Some(&SecretString::from("sk-valid")));
    assert_eq!(
        validated.get("api-key").map(String::as_str),
        Some("sk-valid")
    );
    assert_eq!(
        validated.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        validated.get("Authorization").map(String::as_str),
        Some("Bearer old")
    );
    assert_eq!(validated.get("X-Test").map(String::as_str), Some("kept"));
    assert_eq!(original.get("api-key").map(String::as_str), Some("stale"));

    let blank = validate_environment(&Headers::new(), None);
    assert_eq!(blank.len(), 2);
    assert_eq!(blank.get("api-key").map(String::as_str), Some(""));
    assert_eq!(
        blank.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

fn call_headers() {
    assert_eq!(
        realtime_calls_headers("eph-1"),
        Headers::from([(String::from("api-key"), String::from("eph-1"))])
    );
}

fn ssl_forwarding() {
    let ca_file = Verification::CaFile(String::from("/etc/ca.pem"));
    assert_eq!(ssl_config(&Verification::Enabled), RealtimeSsl::Shared);
    assert_eq!(ssl_config(&Verification::Disabled), RealtimeSsl::Insecure);
    assert_eq!(ssl_config(&ca_file), RealtimeSsl::Shared);

    let secure = connection(Some("https://example.test"), None, Some("GA"), None)
        .with_verification(Verification::Disabled);
    assert_eq!(secure.plan().ssl, RealtimeSsl::Insecure);
    assert_eq!(secure.plan().max_message_size_bytes, MAX_SIZE);

    let plain = connection(Some("http://example.test"), None, Some("GA"), None)
        .with_verification(Verification::Disabled);
    assert_eq!(plain.plan().ssl, RealtimeSsl::Insecure);
}

pub(super) fn handshake_error() {
    handshake_refusals();
    other_failures();
    preparation_refusals();
}

fn handshake_refusals() {
    for status_code in [401_u16, 403, 429, 500, 418] {
        let action = client_action(RealtimeFailure::Handshake { status_code });
        assert_eq!(action, ClientAction::RefuseHandshake { status_code });
    }
}

fn other_failures() {
    assert_eq!(SESSION_ERROR_TYPE, "server_error");
    assert_eq!(SESSION_ERROR_MESSAGE, "Internal server error");
    assert_eq!(SESSION_CLOSE_CODE, 1011);
    assert_eq!(
        client_action(RealtimeFailure::Other),
        ClientAction::SendErrorAndClose {
            error_type: "server_error",
            message: "Internal server error",
            close: RealtimeClose {
                code: 1011,
                reason: String::from("Internal server error"),
            },
        }
    );
}

fn preparation_refusals() {
    let missing_base = connection(None, Some(API_VERSION), None, None).with_key(None);
    assert_eq!(missing_base.error(), RealtimeError::MissingApiBase);
    let missing_version = connection(Some("https://example.test"), None, None, None).with_key(None);
    assert_eq!(missing_version.error(), RealtimeError::MissingApiVersion);
    let missing_key =
        connection(Some("https://example.test"), None, Some("GA"), None).with_key(None);
    assert_eq!(missing_key.error(), RealtimeError::MissingCredentials);
}

pub(super) fn http_version_2024_12_17() {
    base_precedence();
    key_precedence();
    version_precedence();
    session_suffixes();
}

fn base_precedence() {
    let all = http_settings(
        Some("https://global.test"),
        Some("sk-global"),
        Some("https://env.test"),
        Some("sk-env"),
    );
    assert_eq!(
        all.api_base(Some("https://explicit.test")),
        "https://explicit.test"
    );
    assert_eq!(all.api_base(None), "https://global.test");

    let env_only = http_settings(None, None, Some("https://env.test"), None);
    assert_eq!(env_only.api_base(None), "https://env.test");

    let none = http_settings(None, None, None, None);
    assert_eq!(none.api_base(None), "");
}

fn key_precedence() {
    let all = http_settings(
        None,
        Some("sk-global"),
        Some("https://env.test"),
        Some("sk-env"),
    );
    assert_eq!(
        all.api_key(Some("sk-explicit")).expose_secret(),
        "sk-explicit"
    );
    assert_eq!(all.api_key(None).expose_secret(), "sk-global");

    let env_only = http_settings(None, None, None, Some("sk-env"));
    assert_eq!(env_only.api_key(None).expose_secret(), "sk-env");

    let none = http_settings(None, None, None, None);
    assert_eq!(none.api_key(None).expose_secret(), "");
}

fn version_precedence() {
    assert_eq!(DEFAULT_API_VERSION, "2024-12-17");
    let defaulted: RealtimeHttpSettings = http_settings(None, None, None, None);
    assert_eq!(
        defaulted.complete_url(Some("https://x"), None),
        "https://x/openai/realtime/client_secrets?api-version=2024-12-17"
    );

    let versioned = with_version(http_settings(None, None, None, None), Some("v-env"));
    assert_eq!(
        versioned.complete_url(Some("https://x"), Some("2025-01-01")),
        "https://x/openai/realtime/client_secrets?api-version=2025-01-01"
    );
    assert_eq!(
        versioned.complete_url(Some("https://x"), None),
        "https://x/openai/realtime/client_secrets?api-version=v-env"
    );
    assert_eq!(
        versioned.calls_url(Some("https://x"), Some("")),
        "https://x/openai/realtime/calls?api-version="
    );
}

fn session_suffixes() {
    let base = "https://global.test///";
    let settings = http_settings(Some(base), None, None, None);
    assert_eq!(settings.api_base(None), base);
    assert_eq!(
        settings.transcription_session_url(None, None),
        "https://global.test/openai/realtime/transcription_sessions?api-version=2024-12-17"
    );
    assert_eq!(
        settings.calls_url(None, None),
        "https://global.test/openai/realtime/calls?api-version=2024-12-17"
    );
    assert_eq!(
        settings.complete_url(None, None),
        "https://global.test/openai/realtime/client_secrets?api-version=2024-12-17"
    );
}

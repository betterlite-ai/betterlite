use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::openai_realtime::{
    DEFAULT_MAX_MESSAGE_SIZE_BYTES, MAX_MESSAGE_SIZE_VARIABLE, Settings,
};
use betterlite_config::tls::Verification;
use betterlite_openai::realtime::failure::RealtimeClose;
use betterlite_openai::realtime::message_cap::{
    MESSAGE_TOO_BIG_CODE, MESSAGE_TOO_BIG_REASON, oversized_message_close,
};
use secrecy::SecretString;

use crate::realtime::connect::{RealtimeConnectSettings, prepare_connection};

const OVERRIDE_CAP: usize = 1024;

fn load(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("the empty config loads")
        .sections
        .openai_realtime
}

fn plan_cap(max_message_size_bytes: usize) -> usize {
    let key = SecretString::from(String::from("azure-key"));
    let connect = RealtimeConnectSettings {
        api_base: Some("https://example.test"),
        api_key: Some(&key),
        azure_ad_token: None,
        model: "realtime-a",
        api_version: Some("2024-10-01-preview"),
        realtime_protocol: None,
        query_params: None,
        ssl_verification: &Verification::Enabled,
        max_message_size_bytes,
    };
    prepare_connection(&connect)
        .expect("the connection plan is built")
        .max_message_size_bytes
}

fn assert_default_cap() {
    let configured = load(&[]);
    assert_eq!(
        configured.max_message_size_bytes,
        DEFAULT_MAX_MESSAGE_SIZE_BYTES
    );
    assert_eq!(DEFAULT_MAX_MESSAGE_SIZE_BYTES, 16 * 1024 * 1024);
    let cap = plan_cap(configured.max_message_size_bytes);
    assert_eq!(cap, DEFAULT_MAX_MESSAGE_SIZE_BYTES);

    assert_eq!(
        oversized_message_close(DEFAULT_MAX_MESSAGE_SIZE_BYTES, cap),
        None
    );
    assert_eq!(
        oversized_message_close(DEFAULT_MAX_MESSAGE_SIZE_BYTES - 1, cap),
        None
    );
    let close = oversized_message_close(DEFAULT_MAX_MESSAGE_SIZE_BYTES + 1, cap)
        .expect("a message past the default cap closes the socket");
    assert_eq!(
        close,
        RealtimeClose {
            code: MESSAGE_TOO_BIG_CODE,
            reason: String::from(MESSAGE_TOO_BIG_REASON),
        }
    );
    assert_eq!(close.code, 1009);
    assert_eq!(close.reason, "message too big");
}

fn assert_override_cap() {
    let configured = load(&[(MAX_MESSAGE_SIZE_VARIABLE, "1024")]);
    assert_eq!(configured.max_message_size_bytes, OVERRIDE_CAP);
    let cap = plan_cap(configured.max_message_size_bytes);
    assert_eq!(cap, OVERRIDE_CAP);

    assert_eq!(oversized_message_close(OVERRIDE_CAP, cap), None);
    let close = oversized_message_close(OVERRIDE_CAP + 1, cap)
        .expect("a message past the override cap closes the socket");
    assert_eq!(close.code, MESSAGE_TOO_BIG_CODE);
    assert_eq!(close.reason, MESSAGE_TOO_BIG_REASON);
}

#[test]
fn azure_realtime_message_cap_closes_1009() {
    assert_default_cap();
    assert_override_cap();
}

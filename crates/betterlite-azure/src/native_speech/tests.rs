#![cfg(test)]

mod native_speech_cases;
mod support;

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use crate::native_speech::endpoint::get_complete_url;
use crate::native_speech::headers::validate_environment;
use crate::native_speech::identity::NativeSpeechIdentity;
use crate::native_speech::mapping::convert_speed_to_rate;

#[test]
fn azure_native_speech_credential_identity_isolation() {
    let first = support::identity(
        Some("https://first.api.cognitive.microsoft.com"),
        "first-identity-key",
    );
    let second = support::identity(
        Some("https://eastus.api.cognitive.microsoft.com"),
        "second-identity-key",
    );
    let first_headers = validate_environment(&first, BTreeMap::new());
    let second_headers = validate_environment(&second, BTreeMap::new());
    assert_eq!(
        first_headers
            .get("Ocp-Apim-Subscription-Key")
            .map(String::as_str),
        Some("first-identity-key")
    );
    assert_eq!(
        second_headers
            .get("Ocp-Apim-Subscription-Key")
            .map(String::as_str),
        Some("second-identity-key")
    );
    assert_ne!(first_headers, second_headers);
    assert_eq!(
        get_complete_url(&first).unwrap(),
        "https://first.tts.speech.microsoft.com/cognitiveservices/v1"
    );
    assert_eq!(
        get_complete_url(&second).unwrap(),
        "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1"
    );
    let mut cache = BTreeMap::new();
    for identity in [&first, &second] {
        let base = identity.api_base.clone().expect("configured base");
        let key = identity
            .api_key
            .as_ref()
            .map(|value| value.expose_secret().to_owned())
            .expect("configured key");
        cache.insert((base, key), validate_environment(identity, BTreeMap::new()));
    }
    assert_eq!(cache.len(), 2);
    let first_entries = cache
        .values()
        .filter(|headers| {
            headers.get("Ocp-Apim-Subscription-Key").map(String::as_str)
                == Some("first-identity-key")
        })
        .count();
    assert_eq!(first_entries, 1);
}

#[test]
fn azure_native_speech_secret_safe_failure() {
    let identity = NativeSpeechIdentity {
        api_base: None,
        api_key: Some(SecretString::from("fixture-api-key")),
    };
    assert!(!format!("{identity:?}").contains("fixture-api-key"));
    let missing = get_complete_url(&identity).unwrap_err();
    let message = missing.to_string();
    assert!(message.contains("api.cognitive.microsoft.com"));
    assert!(message.contains("tts.speech.microsoft.com"));
    assert!(!message.contains("fixture-api-key"));
    assert!(!format!("{missing:?}").contains("fixture-api-key"));
    let speed = convert_speed_to_rate(f64::NAN).unwrap_err();
    assert!(!format!("{speed}").contains("fixture-api-key"));
    assert!(!format!("{speed:?}").contains("fixture-api-key"));
}

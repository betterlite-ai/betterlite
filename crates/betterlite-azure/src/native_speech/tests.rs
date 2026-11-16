#![cfg(test)]

mod native_speech_cases;
mod support;

use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::native_speech::endpoint::get_complete_url;
use crate::native_speech::headers::validate_environment;
use crate::native_speech::identity::NativeSpeechIdentity;
use crate::native_speech::mapping::convert_speed_to_rate;

#[test]
fn azure_native_speech_credential_identity_isolation() {
    let first = support::identity(
        Some("https://first.api.cognitive.microsoft.com"),
        "first-key",
    );
    let second = support::identity(
        Some("https://eastus.api.cognitive.microsoft.com"),
        "second-key",
    );
    let first_headers = validate_environment(&first, BTreeMap::new());
    let second_headers = validate_environment(&second, BTreeMap::new());
    assert_eq!(support::subscription_key(&first_headers), Some("first-key"));
    assert_eq!(
        support::subscription_key(&second_headers),
        Some("second-key")
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
    let cache = support::identity_cache(&[&first, &second]);
    assert_eq!(cache.len(), 2);
    let cached = cache.get(&support::cache_key(&first)).expect("first entry");
    assert_eq!(support::subscription_key(cached), Some("first-key"));
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

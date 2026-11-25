#![cfg(test)]

use super::prepare;
use crate::audio::convert::{SpeechCallWire, parse_speech};
use crate::audio::session::AudioSession;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use std::collections::BTreeMap;

fn session() -> AudioSession {
    AudioSession {
        environment: BTreeMap::new(),
        client: crate::completion::setup::outbound_client(&BTreeMap::new()).unwrap(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
    }
}

fn wire(arguments: &str) -> SpeechCallWire {
    parse_speech(&format!(r#"{{"arguments":{arguments}}}"#)).unwrap()
}

#[test]
fn speech_prepare_accepts_the_openai_provider() {
    let call = wire(
        r#"{"model":"openai/tts-1","input":"hi","voice":"alloy",
            "api_key":"sk-test","custom_llm_provider":"openai"}"#,
    );
    let prepared = prepare(&session(), &call).unwrap();
    assert_eq!(prepared.model, "tts-1");
    assert_eq!(prepared.provider, "openai");
    assert_eq!(prepared.voice, "alloy");
    assert!(prepared.values.timeout.is_none());
    assert_eq!(prepared.connection.max_retries, 2);
    assert_eq!(
        prepared.connection.api_base.as_deref(),
        Some("https://api.openai.com/v1")
    );
}

#[test]
fn speech_prepare_rejects_another_provider() {
    let call =
        wire(r#"{"model":"vertex_ai/tts","input":"hi","voice":"alloy","api_key":"sk-test"}"#);
    let failure = prepare(&session(), &call).unwrap_err();
    assert!(
        failure
            .message
            .contains("Unable to map the custom llm provider=vertex_ai")
    );
}

#[test]
fn speech_prepare_requires_a_voice() {
    let call = wire(r#"{"model":"tts-1","input":"hi","api_key":"sk-test"}"#);
    let failure = prepare(&session(), &call).unwrap_err();
    assert!(failure.message.contains("'voice' is required"));
}

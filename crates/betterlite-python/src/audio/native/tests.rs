#![cfg(test)]

use pyo3::prelude::*;

use super::NativeAudioSession;
use crate::audio::register;

const SETTINGS: &str = r#"{}"#;

#[test]
fn register_adds_the_audio_classes() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeAudioSession").unwrap());
        assert!(module.hasattr("NativeAudioError").unwrap());
    });
}

#[test]
fn native_audio_session_rejects_a_malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        let session = NativeAudioSession::new(SETTINGS).unwrap();
        assert!(session.transcription(py, "not json", Vec::new()).is_err());
        assert!(session.atranscription(py, "not json", Vec::new()).is_err());
        assert!(session.speech(py, "not json", Vec::new()).is_err());
        assert!(session.aspeech(py, "not json", Vec::new()).is_err());
    });
}

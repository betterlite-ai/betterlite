#![cfg(test)]

use pyo3::prelude::*;

use super::NativeAudioSession;
use crate::audio::answer;
use crate::audio::convert::{SpeechAnswer, SpeechHeader};
use crate::audio::register;
use indexmap::IndexMap;

const SETTINGS: &str = "{}";

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

#[test]
fn request_speech_answer_preserves_audio_and_headers() {
    let result = answer(Ok(SpeechAnswer {
        content: vec![0, 1, 255],
        header: SpeechHeader {
            status_code: 206,
            headers: IndexMap::from([("content-type".to_owned(), "audio/mpeg".to_owned())]),
        },
    }))
    .unwrap();

    assert_eq!(result.0, [0, 1, 255]);
    let header: serde_json::Value = serde_json::from_str(&result.1).unwrap();
    assert_eq!(header["status_code"], 206);
    assert_eq!(header["headers"]["content-type"], "audio/mpeg");
}

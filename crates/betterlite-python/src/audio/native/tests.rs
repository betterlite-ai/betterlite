#![cfg(test)]

use pyo3::prelude::*;

use super::{NativeAudioSession, answer};
use crate::audio::convert::{SpeechAnswer, SpeechHeader};
use crate::audio::register;
use indexmap::IndexMap;

const SETTINGS: &str = "{}";

#[derive(serde::Deserialize)]
struct Header {
    status_code: u16,
    headers: IndexMap<String, String>,
}

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
        session
            .transcription(py, "not json", Vec::new())
            .unwrap_err();
        session
            .atranscription(py, "not json", Vec::new())
            .unwrap_err();
        session.speech(py, "not json", Vec::new()).unwrap_err();
        session.aspeech(py, "not json", Vec::new()).unwrap_err();
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
    let header: Header = serde_json::from_str(&result.1).unwrap();
    assert_eq!(header.status_code, 206);
    assert_eq!(header.headers.get("content-type").unwrap(), "audio/mpeg");
}

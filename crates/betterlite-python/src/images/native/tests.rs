#![cfg(test)]

use super::super::{NativeImageError, register};
use super::{NativeImageSession, encoded};
use crate::images::convert::ImageAnswer;
use crate::images::error::CallFailure;
use betterlite_openai::image_generation::reply::{ImageGenerationReply, ImageUsage};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct EncodedReply {
    created: i64,
    hidden_params: HiddenParams,
}

#[derive(Deserialize)]
struct HiddenParams {
    model: String,
}

fn session() -> NativeImageSession {
    NativeImageSession::new("{}").unwrap()
}

fn malformed_payload_error(py: Python<'_>, error: &PyErr) {
    assert!(error.is_instance_of::<PyValueError>(py));
    assert!(error.to_string().contains("expected shape"));
}

#[test]
fn register_adds_native_image_types() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "image_probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("NativeImageSession").unwrap());
        assert!(module.hasattr("NativeImageError").unwrap());
    });
}

#[test]
fn generate_rejects_malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        malformed_payload_error(py, &session().generate(py, "{", Vec::new()).unwrap_err());
    });
}

#[test]
fn agenerate_rejects_malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        malformed_payload_error(py, &session().agenerate(py, "{", Vec::new()).unwrap_err());
    });
}

#[test]
fn variation_rejects_malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        malformed_payload_error(py, &session().variation(py, "{", Vec::new()).unwrap_err());
    });
}

#[test]
fn avariation_rejects_malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        malformed_payload_error(py, &session().avariation(py, "{", Vec::new()).unwrap_err());
    });
}

#[test]
fn edit_rejects_malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        malformed_payload_error(py, &session().edit(py, "{", Vec::new()).unwrap_err());
    });
}

#[test]
fn aedit_rejects_malformed_payload() {
    Python::initialize();
    Python::attach(|py| {
        malformed_payload_error(py, &session().aedit(py, "{", Vec::new()).unwrap_err());
    });
}

#[test]
fn encoded_preserves_image_reply_fields() {
    let reply = ImageGenerationReply {
        created: 7,
        data: Vec::new(),
        usage: ImageUsage::default(),
        background: None,
        quality: None,
        output_format: None,
        size: None,
    };
    let text = encoded(Ok(ImageAnswer::new(reply, Some("dall-e-2")))).unwrap();
    let value: EncodedReply = serde_json::from_str(&text).unwrap();
    assert_eq!(value.created, 7);
    assert_eq!(value.hidden_params.model, "dall-e-2");
}

#[test]
fn encoded_maps_call_failure_to_native_exception() {
    Python::initialize();
    Python::attach(|py| {
        let error = encoded(Err(CallFailure::plain(
            "ValueError",
            String::from("missing image"),
            None,
        )))
        .unwrap_err();
        assert!(error.is_instance_of::<NativeImageError>(py));
        assert!(error.to_string().contains("missing image"));
    });
}

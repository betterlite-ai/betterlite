#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;
use tracing::subscriber::with_default;

use super::extract_grounding_documents;
use crate::xecguard_primitives::wire::GroundingDocument;

fn request(wire: &str) -> BTreeMap<String, GuardrailValue> {
    serde_json::from_str(wire).expect("request data")
}

fn document(document_id: &str, context: &str) -> GroundingDocument {
    GroundingDocument {
        document_id: document_id.to_owned(),
        context: context.to_owned(),
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn grounding_documents_reads_the_metadata_key() {
    let data = request(
        r#"{"metadata":{"xecguard_grounding_documents":[{"document_id":"d1","context":"ctx","extra":"dropped"}]}}"#,
    );
    assert_eq!(
        extract_grounding_documents(&data),
        vec![document("d1", "ctx")]
    );
}

#[test]
fn grounding_documents_falls_back_to_litellm_metadata() {
    let empty_metadata = request(
        r#"{"metadata":{},"litellm_metadata":{"xecguard_grounding_documents":[{"document_id":"d2","context":"ctx"}]}}"#,
    );
    assert_eq!(
        extract_grounding_documents(&empty_metadata),
        vec![document("d2", "ctx")]
    );
    let non_object_metadata = request(
        r#"{"metadata":"x","litellm_metadata":{"xecguard_grounding_documents":[{"document_id":"d3","context":"ctx"}]}}"#,
    );
    assert_eq!(
        extract_grounding_documents(&non_object_metadata),
        vec![document("d3", "ctx")]
    );
}

#[test]
fn grounding_documents_drops_malformed_documents() {
    let data = request(
        r#"{"metadata":{"xecguard_grounding_documents":[{"document_id":"d1"},{"context":"ctx"},{"document_id":1,"context":"ctx"},{"document_id":"d2","context":null},"bad",[1]]}}"#,
    );
    assert!(extract_grounding_documents(&data).is_empty());
}

#[test]
fn grounding_documents_accepts_empty_strings_that_litellm_forwards() {
    let data = request(
        r#"{"metadata":{"xecguard_grounding_documents":[{"document_id":"","context":""}]}}"#,
    );
    assert_eq!(extract_grounding_documents(&data), vec![document("", "")]);
}

#[rstest]
#[case::missing_request_data("{}")]
#[case::empty_list(r#"{"metadata":{"xecguard_grounding_documents":[]}}"#)]
#[case::non_list(r#"{"metadata":{"xecguard_grounding_documents":{}}}"#)]
#[case::null_metadata(r#"{"metadata":null}"#)]
fn grounding_documents_returns_empty_without_a_document_list(#[case] wire: &str) {
    assert!(extract_grounding_documents(&request(wire)).is_empty());
}

#[test]
fn grounding_documents_logs_each_dropped_document() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let data = request(
        r#"{"metadata":{"xecguard_grounding_documents":[{"document_id":"d1"},{"document_id":"d2","context":"ctx"}]}}"#,
    );
    let documents = with_default(subscriber, || extract_grounding_documents(&data));
    assert_eq!(documents, vec![document("d2", "ctx")]);
    let text =
        String::from_utf8(captured.0.lock().expect("captured log").clone()).expect("utf8 log");
    assert!(text.contains("dropping malformed grounding document"));
}

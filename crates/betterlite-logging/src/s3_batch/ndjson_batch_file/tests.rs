#![cfg(test)]

use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;

use crate::s3_batch::Element;
use crate::s3_batch::ndjson_batch_file::{
    BatchIdSource, batch_file_elements, batch_file_mode_active,
};
use crate::s3_transport::batch_element::ContentType;

#[derive(Debug)]
struct FixedIds(Mutex<usize>);

impl BatchIdSource for FixedIds {
    fn next_batch_id(&self) -> String {
        let mut next = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = format!("id{}", *next);
        *next = next.saturating_add(1);
        id
    }
}

fn ids() -> FixedIds {
    FixedIds(Mutex::new(0))
}

fn payload(text: &str) -> Json {
    serde_json::from_str(text).expect("the payload is JSON")
}

fn element(key: &str, payload_text: &str, retrying_since: Option<f64>) -> Element {
    Element {
        payload: payload(payload_text),
        s3_object_key: key.to_owned(),
        s3_object_download_filename: "download.json".to_owned(),
        body: None,
        content_type: ContentType::Json,
        retrying_since,
    }
}

fn keys(batch: &[Element]) -> Vec<&str> {
    batch
        .iter()
        .map(|element| element.s3_object_key.as_str())
        .collect()
}

#[test]
fn groups_by_parent() {
    let batch = [
        element("logs/2026-01-02/a.json", r#"{"a":1}"#, None),
        element("logs/2026-01-02/b.json", r#"{"b":2}"#, None),
        element("logs/2026-01-03/c.json", r#"{"c":3}"#, None),
    ];
    let files = batch_file_elements(&batch, "03-04-05", &ids()).expect("the batch files build");
    assert_eq!(
        keys(&files),
        vec![
            "logs/2026-01-02/batch_03-04-05_id0.jsonl",
            "logs/2026-01-03/batch_03-04-05_id1.jsonl"
        ]
    );
    assert_eq!(files[0].body.as_deref(), Some("{\"a\":1}\n{\"b\":2}"));
    assert_eq!(
        files[0].s3_object_download_filename,
        "batch_03-04-05_id0.jsonl"
    );
    assert_eq!(files[0].content_type, ContentType::Ndjson);
    assert_eq!(files[0].payload, payload("{}"));
}

#[test]
fn sorts_parents() {
    let batch = [
        element("z/one.json", r#"{"z":1}"#, None),
        element("a/two.json", r#"{"a":2}"#, None),
        element("m/three.json", r#"{"m":3}"#, None),
    ];
    let files = batch_file_elements(&batch, "00-00-00", &ids()).expect("the batch files build");
    assert_eq!(
        keys(&files),
        vec![
            "a/batch_00-00-00_id0.jsonl",
            "m/batch_00-00-00_id1.jsonl",
            "z/batch_00-00-00_id2.jsonl"
        ]
    );
}

#[test]
fn combines_payloads_to_ndjson() {
    let batch = [
        element("logs/a.json", r#"{"one":1}"#, None),
        element("logs/b.json", r#"{"two":2}"#, None),
        element("logs/c.json", r#"{"three":3}"#, None),
    ];
    let files = batch_file_elements(&batch, "12-00-00", &ids()).expect("the batch files build");
    assert_eq!(
        files[0].body.as_deref(),
        Some("{\"one\":1}\n{\"two\":2}\n{\"three\":3}")
    );
}

#[test]
fn earliest_retrying_since() {
    let batch = [
        element("logs/a.json", r#"{"a":1}"#, Some(30.0)),
        element("logs/b.json", r#"{"b":2}"#, Some(10.0)),
        element("logs/c.json", r#"{"c":3}"#, None),
    ];
    let files = batch_file_elements(&batch, "12-00-00", &ids()).expect("the batch files build");
    assert_eq!(files[0].retrying_since, Some(10.0));

    let fresh = [
        element("logs/a.json", r#"{"a":1}"#, None),
        element("logs/b.json", r#"{"b":2}"#, None),
    ];
    let files = batch_file_elements(&fresh, "12-00-00", &ids()).expect("the batch files build");
    assert_eq!(files[0].retrying_since, None);
}

#[test]
fn keys_files_at_the_root_without_a_parent() {
    let batch = [
        element("a.json", r#"{"a":1}"#, None),
        element("b.json", r#"{"b":2}"#, None),
    ];
    let files = batch_file_elements(&batch, "12-00-00", &ids()).expect("the batch files build");
    assert_eq!(keys(&files), vec!["batch_12-00-00_id0.jsonl"]);
}

#[test]
fn keeps_uploaded_batch_files_out_of_grouping() {
    let mut existing = element("logs/batch_01-00-00_old.jsonl", r#"{"a":1}"#, None);
    existing.body = Some("{\"a\":1}".to_owned());
    existing.content_type = ContentType::Ndjson;
    let batch = [existing, element("logs/b.json", r#"{"b":2}"#, None)];
    let files = batch_file_elements(&batch, "12-00-00", &ids()).expect("the batch files build");
    assert_eq!(
        keys(&files),
        vec![
            "logs/batch_01-00-00_old.jsonl",
            "logs/batch_12-00-00_id0.jsonl"
        ]
    );
}

#[test]
fn disables_when_cold_storage() {
    assert!(!batch_file_mode_active(true, Some("s3_v2")));
    assert!(batch_file_mode_active(true, Some("gcs_bucket")));
    assert!(batch_file_mode_active(true, None));
    assert!(!batch_file_mode_active(false, None));
    assert!(!batch_file_mode_active(false, Some("s3_v2")));
}

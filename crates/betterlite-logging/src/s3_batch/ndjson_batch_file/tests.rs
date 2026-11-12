#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;

use crate::s3_batch::Element;
use crate::s3_batch::ndjson_batch_file::{
    batch_file_elements, batch_file_mode_active, batch_stamp,
};
use crate::s3_transport::batch_element::ContentType;
use crate::s3_transport::signing::Clock;

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
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

fn parents(batch: &[Element]) -> Vec<&str> {
    batch
        .iter()
        .map(|element| {
            element
                .s3_object_key
                .rsplit_once('/')
                .map_or("", |(parent, _name)| parent)
        })
        .collect()
}

#[test]
fn groups_by_parent() {
    let batch = [
        element("logs/2026-01-02/a.json", r#"{"a":1}"#, None),
        element("logs/2026-01-02/b.json", r#"{"b":2}"#, None),
        element("logs/2026-01-03/c.json", r#"{"c":3}"#, None),
    ];
    let files = batch_file_elements(&batch, "03-04-05").expect("the batch files build");
    assert_eq!(parents(&files), vec!["logs/2026-01-02", "logs/2026-01-03"]);
    assert!(files[0].s3_object_key.contains("/batch_03-04-05_"));
    assert_eq!(files[0].s3_object_key.rsplit('.').next(), Some("jsonl"));
    assert_eq!(files[0].body.as_deref(), Some("{\"a\":1}\n{\"b\":2}"));
    let name = &files[0].s3_object_download_filename;
    assert!(name.starts_with("batch_03-04-05_") && name.rsplit('.').next() == Some("jsonl"));
    assert_eq!(name.len(), "batch_03-04-05_".len() + 32 + ".jsonl".len());
    assert_eq!(files[0].s3_object_key, format!("logs/2026-01-02/{name}"));
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
    let files = batch_file_elements(&batch, "00-00-00").expect("the batch files build");
    assert_eq!(parents(&files), vec!["a", "m", "z"]);
}

#[test]
fn combines_payloads_to_ndjson() {
    let batch = [
        element("logs/a.json", r#"{"one":1}"#, None),
        element("logs/b.json", r#"{"two":2}"#, None),
        element("logs/c.json", r#"{"three":3}"#, None),
    ];
    let files = batch_file_elements(&batch, "12-00-00").expect("the batch files build");
    assert_eq!(
        files[0].body.as_deref(),
        Some("{\"one\":1}\n{\"two\":2}\n{\"three\":3}")
    );
}

#[test]
fn earliest_retrying_since() {
    let batch = [
        element("logs/a.json", r#"{"a":1}"#, Some(30.0_f64)),
        element("logs/b.json", r#"{"b":2}"#, Some(10.0_f64)),
        element("logs/c.json", r#"{"c":3}"#, None),
    ];
    let files = batch_file_elements(&batch, "12-00-00").expect("the batch files build");
    assert_eq!(files[0].retrying_since, Some(10.0_f64));

    let fresh = [
        element("logs/a.json", r#"{"a":1}"#, None),
        element("logs/b.json", r#"{"b":2}"#, None),
    ];
    let fresh_files = batch_file_elements(&fresh, "12-00-00").expect("the batch files build");
    assert_eq!(fresh_files[0].retrying_since, None);
}

#[test]
fn keys_files_at_the_root_without_a_parent() {
    let batch = [
        element("a.json", r#"{"a":1}"#, None),
        element("b.json", r#"{"b":2}"#, None),
    ];
    let files = batch_file_elements(&batch, "12-00-00").expect("the batch files build");
    assert!(!files[0].s3_object_key.contains('/'));
    assert_eq!(files[0].s3_object_key.rsplit('.').next(), Some("jsonl"));
}

#[test]
fn keeps_uploaded_batch_files_out_of_grouping() {
    let mut existing = element("logs/batch_01-00-00_old.jsonl", r#"{"a":1}"#, None);
    existing.body = Some("{\"a\":1}".to_owned());
    existing.content_type = ContentType::Ndjson;
    let batch = [existing, element("logs/b.json", r#"{"b":2}"#, None)];
    let files = batch_file_elements(&batch, "12-00-00").expect("the batch files build");
    assert_eq!(files[0].s3_object_key, "logs/batch_01-00-00_old.jsonl");
    assert_eq!(parents(&files), vec!["logs", "logs"]);
}

#[test]
fn disables_when_cold_storage() {
    assert!(!batch_file_mode_active(true, Some("s3_v2")));
    assert!(batch_file_mode_active(true, Some("gcs_bucket")));
    assert!(batch_file_mode_active(true, None));
    assert!(!batch_file_mode_active(false, None));
    assert!(!batch_file_mode_active(false, Some("s3_v2")));
}

#[test]
fn stamps_batch_files_with_the_clock_time() {
    let moment = UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_323_045))
        .expect("the clock fits");
    assert_eq!(batch_stamp(&FixedClock(moment)), "03-04-05");
}

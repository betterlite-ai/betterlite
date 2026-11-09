#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::CacheValue;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::subscriber::with_default;

use super::{CacheCodec, CodecError};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct BudgetModel {
    model: String,
    #[serde(rename = "max_budget")]
    limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration: Option<String>,
}

fn budget(model: &str, limit: Option<f64>, duration: Option<&str>) -> BudgetModel {
    BudgetModel {
        model: model.to_owned(),
        limit,
        duration: duration.map(str::to_owned),
    }
}

fn object(fields: &[(&str, CacheValue)]) -> CacheValue {
    CacheValue::Object(
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn text(value: &str) -> CacheValue {
    CacheValue::Text(value.to_owned())
}

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .flush()
    }
}

fn captured_log<Model>(cached: &CacheValue) -> (Option<Model>, String)
where
    Model: DeserializeOwned,
{
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogCapture(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    let decoded = with_default(subscriber, || CacheCodec::decode(Some(cached)));
    let log = bytes.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (decoded, String::from_utf8(log).unwrap())
}

#[test]
fn codec_serializes_a_model_to_its_json_form() {
    let value = CacheCodec::encode(&budget("gpt-4o", Some(10.5_f64), None)).unwrap();
    assert_eq!(
        value,
        object(&[
            ("model", text("gpt-4o")),
            ("max_budget", CacheValue::Float(10.5)),
        ])
    );
}

#[test]
fn codec_serializes_an_absent_model_field_as_null() {
    let value = CacheCodec::encode(&budget("gpt-4o", None, Some("30d"))).unwrap();
    assert_eq!(
        value,
        object(&[
            ("model", text("gpt-4o")),
            ("max_budget", CacheValue::Null),
            ("duration", text("30d")),
        ])
    );
}

#[test]
fn codec_serialize_passes_an_untyped_value_through() {
    let value = text("plain");
    assert_eq!(CacheCodec::encode(&value).unwrap(), value);
}

#[test]
fn codec_serialize_reports_a_value_that_is_not_json() {
    let stored = CacheCodec::encode(&CacheValue::Bytes(vec![1]));
    assert!(matches!(stored, Err(CodecError::Serialize { .. })));
}

#[test]
fn codec_serialize_validated_dumps_the_validated_fields() {
    let payload = object(&[
        ("model", text("gpt-4o")),
        ("max_budget", CacheValue::Integer(10)),
        ("duration", CacheValue::Null),
    ]);
    let stored = CacheCodec::encode_validated::<BudgetModel>(&payload).unwrap();
    assert_eq!(
        stored,
        object(&[
            ("model", text("gpt-4o")),
            ("max_budget", CacheValue::Float(10.0)),
        ])
    );
}

#[test]
fn codec_serialize_validated_rejects_a_mismatched_payload() {
    let payload = object(&[("model", text("gpt-4o")), ("max_budget", text("ten"))]);
    let stored = CacheCodec::encode_validated::<BudgetModel>(&payload);
    assert!(matches!(stored, Err(CodecError::Validation { .. })));
}

#[test]
fn codec_serialize_validated_rejects_a_missing_field() {
    let payload = object(&[
        ("max_budget", CacheValue::Null),
        ("duration", CacheValue::Null),
    ]);
    let stored = CacheCodec::encode_validated::<BudgetModel>(&payload);
    assert!(matches!(stored, Err(CodecError::Validation { .. })));
}

#[test]
fn codec_serialize_validated_passes_a_non_mapping_through() {
    let stored = CacheCodec::encode_validated::<BudgetModel>(&text("gpt-4o")).unwrap();
    assert_eq!(stored, text("gpt-4o"));
}

#[test]
fn codec_serialize_validated_passes_a_number_through() {
    let stored = CacheCodec::encode_validated::<BudgetModel>(&CacheValue::Integer(5)).unwrap();
    assert_eq!(stored, CacheValue::Integer(5));
}

#[test]
fn codec_deserialize_returns_none_for_an_absent_entry() {
    assert_eq!(CacheCodec::decode::<BudgetModel>(None), None);
}

#[test]
fn codec_deserialize_returns_none_for_a_stored_null() {
    assert_eq!(
        CacheCodec::decode::<BudgetModel>(Some(&CacheValue::Null)),
        None
    );
}

#[test]
fn codec_deserialize_returns_none_for_a_non_mapping_without_a_warning() {
    let (decoded, log) = captured_log::<BudgetModel>(&text("cached"));
    assert_eq!(decoded, None);
    assert!(!log.contains("validation failed"), "{log}");
}

#[test]
fn codec_deserialize_validates_a_mapping() {
    let stored = object(&[
        ("model", text("gpt-4o")),
        ("max_budget", CacheValue::Float(10.5)),
    ]);
    assert_eq!(
        CacheCodec::decode::<BudgetModel>(Some(&stored)),
        Some(budget("gpt-4o", Some(10.5_f64), None))
    );
}

#[test]
fn codec_deserialize_ignores_an_unknown_field() {
    let stored = object(&[
        ("model", text("gpt-4o")),
        ("max_budget", CacheValue::Null),
        ("unknown", text("kept out")),
    ]);
    assert_eq!(
        CacheCodec::decode::<BudgetModel>(Some(&stored)),
        Some(budget("gpt-4o", None, None))
    );
}

#[test]
fn codec_deserialize_warns_and_misses_on_a_schema_mismatch() {
    let stored = object(&[("model", text("gpt-4o")), ("max_budget", text("ten"))]);
    let (decoded, log) = captured_log::<BudgetModel>(&stored);
    assert_eq!(decoded, None);
    assert!(log.contains("validation failed"), "{log}");
    assert!(log.contains("BudgetModel"), "{log}");
}

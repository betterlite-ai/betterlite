use std::fs;
use std::path::PathBuf;

use betterlite_openai::batches::records::{BatchObject, BatchStatus};
use rstest::rstest;

use super::super::decoder::{ManagedBatchRecordError, decode_managed_batch_record};
use super::BATCH_JSON;

#[test]
fn batch_object_and_json_string_validation() {
    let from_object = decode_managed_batch_record::<BatchObject>(BATCH_JSON.as_bytes())
        .unwrap()
        .unwrap();
    let quoted = serde_json::to_string(BATCH_JSON).unwrap();
    let from_string = decode_managed_batch_record::<BatchObject>(quoted.as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(from_object, from_string);
    assert_eq!(from_object.id, "batch_1");
    assert_eq!(from_object.status, BatchStatus::Validating);
}

#[rstest]
#[case::missing_id(r#"{"completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"f","object":"batch","status":"validating"}"#)]
#[case::missing_status(r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"f","object":"batch"}"#)]
#[case::unknown_status(r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"f","object":"batch","status":"done"}"#)]
#[case::wrong_object(r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"f","object":"list","status":"validating"}"#)]
fn batch_required_fields_fail(#[case] stored: &str) {
    assert!(decode_managed_batch_record::<BatchObject>(stored.as_bytes()).is_err());
    let quoted = serde_json::to_string(stored).unwrap();
    assert!(decode_managed_batch_record::<BatchObject>(quoted.as_bytes()).is_err());
}

#[rstest]
#[case::null("null")]
#[case::empty_string(r#""""#)]
#[case::empty_object("{}")]
#[case::empty_array("[]")]
#[case::false_value("false")]
#[case::zero("0")]
#[case::zero_float("0.0")]
fn batch_missing_or_falsey_blob_returns_none(#[case] stored: &str) {
    assert!(
        decode_managed_batch_record::<BatchObject>(stored.as_bytes())
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[case::null_string(r#""null""#)]
#[case::empty_object_string(r#""{}""#)]
fn truthy_null_json_is_validated(#[case] stored: &str) {
    assert!(decode_managed_batch_record::<BatchObject>(stored.as_bytes()).is_err());
}

#[test]
fn native_managed_batch_record_limits() {
    let prefix = r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"f","object":"batch","status":"validating","padding":""#;
    let suffix = "\"}";
    let room = 0x0010_0000_usize - prefix.len() - suffix.len();
    let exact = format!("{prefix}{}{suffix}", "a".repeat(room));
    assert_eq!(exact.len(), 0x0010_0000);
    assert!(
        decode_managed_batch_record::<BatchObject>(exact.as_bytes())
            .unwrap()
            .is_some()
    );
    let over = format!("{exact} ");
    assert!(matches!(
        decode_managed_batch_record::<BatchObject>(over.as_bytes()),
        Err(ManagedBatchRecordError::TooLarge { .. })
    ));

    let nested = |depth: usize| {
        format!(
            r#"{{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"f","object":"batch","status":"validating","extra":{}"a"{}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        )
    };
    let shallow = nested(63);
    assert!(
        decode_managed_batch_record::<BatchObject>(shallow.as_bytes())
            .unwrap()
            .is_some()
    );
    let deep = nested(64);
    assert!(matches!(
        decode_managed_batch_record::<BatchObject>(deep.as_bytes()),
        Err(ManagedBatchRecordError::TooDeep { .. })
    ));
}

#[rstest]
fn replays_the_managed_batch_record_corpus(
    #[files("fuzz/corpus/managed_batch_record/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        decode_managed_batch_record::<BatchObject>(&bytes).is_ok(),
        !name.starts_with("invalid-"),
        "{name}"
    );
}

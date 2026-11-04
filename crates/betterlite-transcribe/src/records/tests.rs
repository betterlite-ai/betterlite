#![cfg(test)]

use crate::records::{TranscriptionJobResponse, TranscriptionJobTag};

#[test]
fn transcription_records_deserialize_aws_casing_and_optional_fields() {
    let response: TranscriptionJobResponse =
        serde_json::from_str(r#"{"TranscriptionJob":{"Tags":[{"Key":"x"}]}}"#).unwrap();
    let job = response.transcription_job.unwrap();
    assert_eq!(
        job.tags,
        vec![TranscriptionJobTag {
            key: Some("x".to_owned()),
            value: None
        }]
    );
    assert_eq!(job.media, None);
}

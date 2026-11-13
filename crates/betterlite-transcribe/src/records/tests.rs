#![cfg(test)]

use crate::records::{JobLookupError, TranscriptionJobResponse, TranscriptionJobTag};

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

#[test]
fn rejected_lookup_carries_the_status_and_the_error_type() {
    let error = JobLookupError::Rejected {
        status: 400,
        aws_error_type: Some("NotFoundException".to_owned()),
    };
    match &error {
        JobLookupError::Rejected {
            status,
            aws_error_type,
        } => {
            assert_eq!(*status, 400);
            assert_eq!(aws_error_type.as_deref(), Some("NotFoundException"));
        }
        JobLookupError::Unavailable | JobLookupError::InvalidResponse(_) => {
            panic!("unexpected variant: {error:?}")
        }
    }
    assert!(error.to_string().contains("400"));
}

#[test]
fn rejected_lookup_keeps_a_rejection_without_a_type() {
    let error = JobLookupError::Rejected {
        status: 503,
        aws_error_type: None,
    };
    match &error {
        JobLookupError::Rejected {
            status,
            aws_error_type,
        } => {
            assert_eq!(*status, 503);
            assert_eq!(*aws_error_type, None);
        }
        JobLookupError::Unavailable | JobLookupError::InvalidResponse(_) => {
            panic!("unexpected variant: {error:?}")
        }
    }
}

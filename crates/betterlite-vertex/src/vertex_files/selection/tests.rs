#![cfg(test)]

use std::sync::Mutex;

use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileBody, FileData};

use crate::vertex_batch_upload::sources::{FileContent, FileTypes};

use super::{BatchJsonlFiles, BatchJsonlPredicate, SelectedUpload, select_upload};
use crate::vertex_files::FileRequest;
use crate::vertex_files::error::VertexFilesError;

#[derive(Debug, PartialEq, Eq)]
struct Observed {
    purpose: String,
    content_type: String,
    filename: Option<String>,
}

// records every question the production code asks, then answers with the test's own choice
struct RecordingPredicate {
    answer: bool,
    observed: Mutex<Vec<Observed>>,
}

impl BatchJsonlPredicate for RecordingPredicate {
    fn matches(&self, purpose: &str, content_type: &str, file: &FileTypes) -> bool {
        let filename = match file {
            FileTypes::Tuple { filename, .. } => Some(filename.clone()),
            FileTypes::Content(_) => None,
        };
        self.observed.lock().unwrap().push(Observed {
            purpose: purpose.to_owned(),
            content_type: content_type.to_owned(),
            filename,
        });
        self.answer
    }
}

#[test]
fn a_passthrough_batch_upload_streams_the_raw_file() {
    let file = FileData::Tuple3 {
        filename: Some("rows.jsonl".to_owned()),
        content: FileBody::Bytes(b"{\"a\":1}\n".to_vec()),
        content_type: Some("application/jsonl".to_owned()),
    };
    let predicate = RecordingPredicate {
        answer: true,
        observed: Mutex::new(Vec::new()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: Some("gemini-2.5-pro"),
            passthrough: true,
            custom_endpoint: false,
        },
        &predicate,
    )
    .unwrap();
    assert_eq!(selected.content_type(), Some("application/json"));
    let SelectedUpload::Passthrough(FileTypes::Tuple { filename, content }) = selected else {
        panic!("a passthrough upload streams the raw tuple");
    };
    assert_eq!(filename, "rows.jsonl");
    let FileContent::Bytes(bytes) = content else {
        panic!("the tuple carries the uploaded bytes");
    };
    assert_eq!(bytes, b"{\"a\":1}\n".to_vec());
    assert!(predicate.observed.lock().unwrap().is_empty());
}

#[test]
fn passthrough_outside_the_batch_purpose_goes_through_the_predicate() {
    let file = FileData::Tuple3 {
        filename: Some("rows.jsonl".to_owned()),
        content: FileBody::Bytes(b"{\"a\":1}\n".to_vec()),
        content_type: Some("application/jsonl".to_owned()),
    };
    let predicate = RecordingPredicate {
        answer: true,
        observed: Mutex::new(Vec::new()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::FineTune,
            deployment_model: Some("gemini-2.5-pro"),
            passthrough: true,
            custom_endpoint: false,
        },
        &predicate,
    )
    .unwrap();
    assert!(matches!(selected, SelectedUpload::Converted(_)));
    assert_eq!(predicate.observed.lock().unwrap().len(), 1);
}

#[test]
fn a_batch_jsonl_upload_is_converted_to_vertex_rows() {
    let file = FileData::Tuple3 {
        filename: Some("rows.jsonl".to_owned()),
        content: FileBody::Bytes(b"{\"a\":1}\n".to_vec()),
        content_type: Some("application/jsonl".to_owned()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &BatchJsonlFiles,
    )
    .unwrap();
    assert_eq!(selected.content_type(), Some("application/json"));
    let SelectedUpload::Converted(FileTypes::Tuple { filename, content }) = selected else {
        panic!("a batch JSONL file is converted");
    };
    assert_eq!(filename, "rows.jsonl");
    let FileContent::Bytes(bytes) = content else {
        panic!("the converted stream reads the uploaded bytes");
    };
    assert_eq!(bytes, b"{\"a\":1}\n".to_vec());
}

#[test]
fn a_batch_file_that_is_not_jsonl_uploads_its_bytes() {
    let file = FileData::Tuple3 {
        filename: Some("report.pdf".to_owned()),
        content: FileBody::Bytes(b"%PDF-1.4".to_vec()),
        content_type: Some("application/pdf".to_owned()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &BatchJsonlFiles,
    )
    .unwrap();
    assert_eq!(selected.content_type(), None);
    let SelectedUpload::Bytes(bytes) = selected else {
        panic!("a file that is not JSONL is uploaded as it arrived");
    };
    assert_eq!(bytes, b"%PDF-1.4".to_vec());
}

#[test]
fn an_ordinary_purpose_uploads_its_bytes() {
    let file = FileData::Tuple3 {
        filename: Some("rows.jsonl".to_owned()),
        content: FileBody::Bytes(b"{\"a\":1}\n".to_vec()),
        content_type: Some("application/jsonl".to_owned()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Assistants,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &BatchJsonlFiles,
    )
    .unwrap();
    assert_eq!(selected.content_type(), None);
    assert!(matches!(selected, SelectedUpload::Bytes(_)));
}

#[test]
fn the_predicate_sees_the_purpose_and_the_declared_mime() {
    let file = FileData::Tuple3 {
        filename: Some("rows.jsonl".to_owned()),
        content: FileBody::Bytes(b"{\"a\":1}\n".to_vec()),
        content_type: Some("application/jsonl".to_owned()),
    };
    let predicate = RecordingPredicate {
        answer: false,
        observed: Mutex::new(Vec::new()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &predicate,
    )
    .unwrap();
    assert!(matches!(selected, SelectedUpload::Bytes(_)));
    assert_eq!(
        *predicate.observed.lock().unwrap(),
        vec![Observed {
            purpose: "batch".to_owned(),
            content_type: "application/jsonl".to_owned(),
            filename: Some("rows.jsonl".to_owned()),
        }]
    );
}

#[test]
fn the_filename_mime_reaches_the_predicate() {
    let file = FileData::Tuple2 {
        filename: Some("rows.jsonl".to_owned()),
        content: FileBody::Bytes(b"{\"a\":1}\n".to_vec()),
    };
    let predicate = RecordingPredicate {
        answer: true,
        observed: Mutex::new(Vec::new()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &predicate,
    )
    .unwrap();
    assert!(matches!(selected, SelectedUpload::Converted(_)));
    assert_eq!(
        *predicate.observed.lock().unwrap(),
        vec![Observed {
            purpose: "batch".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            filename: Some("rows.jsonl".to_owned()),
        }]
    );
}

#[test]
fn a_direct_upload_reaches_the_predicate_without_a_filename() {
    let file = FileData::Direct(FileBody::Bytes(b"plain".to_vec()));
    let predicate = RecordingPredicate {
        answer: false,
        observed: Mutex::new(Vec::new()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &predicate,
    )
    .unwrap();
    assert!(matches!(selected, SelectedUpload::Bytes(_)));
    assert_eq!(
        *predicate.observed.lock().unwrap(),
        vec![Observed {
            purpose: "batch".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            filename: None,
        }]
    );
}

#[test]
fn a_string_body_is_not_an_upload_stream() {
    let file = FileData::Tuple2 {
        filename: Some("rows.jsonl".to_owned()),
        content: FileBody::BareString("{\"a\":1}\n".to_owned()),
    };
    let failure = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Assistants,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &BatchJsonlFiles,
    )
    .expect_err("the source only uploads bytes");
    assert!(matches!(failure, VertexFilesError::Media(_)));
}

#[test]
fn an_empty_filename_reaches_the_predicate_without_a_name() {
    let file = FileData::Tuple2 {
        filename: Some(String::new()),
        content: FileBody::Bytes(b"{\"a\":1}\n".to_vec()),
    };
    let predicate = RecordingPredicate {
        answer: false,
        observed: Mutex::new(Vec::new()),
    };
    let selected = select_upload(
        &FileRequest {
            file: &file,
            purpose: CreatePurpose::Batch,
            deployment_model: None,
            passthrough: false,
            custom_endpoint: false,
        },
        &predicate,
    )
    .unwrap();
    assert!(matches!(selected, SelectedUpload::Bytes(_)));
    assert_eq!(
        *predicate.observed.lock().unwrap(),
        vec![Observed {
            purpose: "batch".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            filename: None,
        }]
    );
}

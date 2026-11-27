#![cfg(test)]

use std::time::Duration;

use indexmap::IndexMap;

use crate::api::batch_records::{BatchIdRequest, CreateBatchRequest};
use crate::batches::records::{
    BatchCreate, BatchEndpoint, BatchQuery, CompletionWindow, ExpiryAnchor, MissingBatchField,
    OutputExpiry,
};
use crate::batches::sdk::{Timeout, TimeoutError, TimeoutPhases};

#[test]
fn create_from_request_carries_the_fields() {
    let body = CreateBatchRequest {
        completion_window: Some(CompletionWindow::Day),
        endpoint: Some(BatchEndpoint::ChatCompletions),
        input_file_id: Some("file-1".to_owned()),
        metadata: Some(IndexMap::from([("k".to_owned(), "v".to_owned())])),
        output_expires_after: Some(OutputExpiry {
            anchor: ExpiryAnchor::CreatedAt,
            seconds: 60,
        }),
        extra_headers: None,
        extra_body: None,
        timeout: Some(12.5_f64),
    };
    let request = BatchCreate::from_request(&body).unwrap();
    assert_eq!(request.completion_window, CompletionWindow::Day);
    assert_eq!(request.endpoint, BatchEndpoint::ChatCompletions);
    assert_eq!(request.input_file_id, "file-1");
    assert_eq!(
        request
            .metadata
            .as_ref()
            .unwrap()
            .get("k")
            .map(String::as_str),
        Some("v")
    );
    assert_eq!(
        request.output_expires_after,
        Some(OutputExpiry {
            anchor: ExpiryAnchor::CreatedAt,
            seconds: 60,
        })
    );
    assert_eq!(request.timeout, Some(12.5_f64));
    assert!(request.extra_headers.is_none());
    assert!(request.extra_body.is_none());
}

#[test]
fn create_from_request_names_the_missing_field() {
    let error = BatchCreate::from_request(&CreateBatchRequest::default()).unwrap_err();
    assert_eq!(
        error,
        MissingBatchField::Missing {
            field: "completion_window"
        }
    );
    assert_eq!(
        error.to_string(),
        "the batch request is missing its completion_window"
    );
    let no_endpoint = CreateBatchRequest {
        completion_window: Some(CompletionWindow::Day),
        endpoint: None,
        input_file_id: None,
        metadata: None,
        output_expires_after: None,
        extra_headers: None,
        extra_body: None,
        timeout: None,
    };
    assert_eq!(
        BatchCreate::from_request(&no_endpoint).unwrap_err(),
        MissingBatchField::Missing { field: "endpoint" }
    );
    let no_file = CreateBatchRequest {
        completion_window: Some(CompletionWindow::Day),
        endpoint: Some(BatchEndpoint::Embeddings),
        input_file_id: None,
        metadata: None,
        output_expires_after: None,
        extra_headers: None,
        extra_body: None,
        timeout: None,
    };
    assert_eq!(
        BatchCreate::from_request(&no_file).unwrap_err(),
        MissingBatchField::Missing {
            field: "input_file_id"
        }
    );
}

#[test]
fn query_from_request_carries_the_batch_id() {
    let request = BatchQuery::from_request(&BatchIdRequest {
        batch_id: Some("batch-1".to_owned()),
        extra_headers: Some(IndexMap::from([("h".to_owned(), "v".to_owned())])),
        extra_body: None,
        timeout: Some(3.0_f64),
    })
    .unwrap();
    assert_eq!(request.batch_id, "batch-1");
    assert_eq!(
        request
            .extra_headers
            .as_ref()
            .unwrap()
            .get("h")
            .map(String::as_str),
        Some("v")
    );
    assert_eq!(request.timeout, Some(3.0_f64));
    let missing = BatchQuery::from_request(&BatchIdRequest::default()).unwrap_err();
    assert_eq!(missing, MissingBatchField::Missing { field: "batch_id" });
    assert_eq!(
        missing.to_string(),
        "the batch request is missing its batch_id"
    );
}

#[test]
fn timeout_duration_follows_the_pieces() {
    assert_eq!(
        Timeout::Seconds(30.0).duration().unwrap(),
        Duration::from_secs(30)
    );
    let read = TimeoutPhases {
        connect: None,
        read: Some(5.0_f64),
        write: Some(9.0_f64),
        pool: None,
    };
    assert_eq!(
        Timeout::Phases(read).duration().unwrap(),
        Duration::from_secs(5)
    );
    let bare = TimeoutPhases {
        connect: None,
        read: None,
        write: None,
        pool: None,
    };
    assert_eq!(
        Timeout::Phases(bare).duration().unwrap(),
        Duration::from_secs(600)
    );
    assert_eq!(
        Timeout::Seconds(-1.0).duration().unwrap_err(),
        TimeoutError::Invalid { seconds: -1.0_f64 }
    );
    assert_eq!(
        Timeout::Seconds(-1.0).duration().unwrap_err().to_string(),
        "the timeout must be a finite, non-negative number of seconds, not -1"
    );
    let negative_read = TimeoutPhases {
        connect: Some(-2.0_f64),
        read: Some(-3.0_f64),
        write: None,
        pool: None,
    };
    assert_eq!(
        Timeout::Phases(negative_read).duration().unwrap_err(),
        TimeoutError::Invalid { seconds: -3.0_f64 }
    );
}

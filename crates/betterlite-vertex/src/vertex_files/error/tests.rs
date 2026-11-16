#![cfg(test)]

use super::VertexFilesError;

#[test]
fn only_the_two_batch_refusals_are_client_errors() {
    assert_eq!(VertexFilesError::PassthroughModel.status_code(), Some(400));
    assert_eq!(
        VertexFilesError::CustomEndpointBatch.status_code(),
        Some(400)
    );
    assert_eq!(VertexFilesError::MissingApiKey.status_code(), None);
    assert_eq!(VertexFilesError::Listing.status_code(), None);
}

#[test]
fn listing_reports_the_source_unsupported_message() {
    assert_eq!(
        VertexFilesError::Listing.to_string(),
        "VertexAIFilesConfig does not support file listing"
    );
}

#[test]
fn the_auth_error_keeps_the_source_message() {
    assert_eq!(
        VertexFilesError::MissingApiKey.to_string(),
        "api_key is required"
    );
}

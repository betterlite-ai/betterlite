use percent_encoding::utf8_percent_encode;
use sha2::{Digest as _, Sha256};

use super::CallRequest;
use crate::bedrock_ingestion::client::S3_SERVICE;
use crate::bedrock_ingestion::client::endpoints::s3_bucket_host;

pub const PUT_OBJECT: &str = "PutObject";
pub const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";

#[must_use]
pub fn object_key(key: &str) -> String {
    key.split('/')
        .map(|segment| utf8_percent_encode(segment, super::UNRESERVED).to_string())
        .collect::<Vec<String>>()
        .join("/")
}

#[must_use]
pub fn put_object(
    region: &str,
    bucket: &str,
    key: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> CallRequest {
    let content_type = content_type
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CONTENT_TYPE);
    CallRequest {
        operation: PUT_OBJECT,
        service: S3_SERVICE,
        signing_region: region.to_owned(),
        method: "PUT",
        url: format!("{}/{}", s3_bucket_host(bucket, region), object_key(key)),
        headers: vec![
            ("Content-Type".to_owned(), content_type.to_owned()),
            (
                "x-amz-content-sha256".to_owned(),
                hex::encode(Sha256::digest(body)),
            ),
        ],
        body: body.to_vec(),
    }
}

mod tests;

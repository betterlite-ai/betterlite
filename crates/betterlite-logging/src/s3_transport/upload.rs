use std::str::from_utf8;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::Method;
use md5::Md5;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tokio::time::sleep;

use crate::s3_transport::batch_element::{ContentType, S3BatchLoggingElement};
use crate::s3_transport::encryption::ServerSideEncryption;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::error_code::{error_code, is_terminal};
use crate::s3_transport::signing::{RequestToSign, sign_request};
use crate::s3_transport::transport::{S3Request, S3Response, S3Transport, UploadOutcome};

mod tests;

const MAX_ATTEMPTS: u32 = 3;
const LAST_ATTEMPT: u32 = MAX_ATTEMPTS - 1;
const CONTENT_LANGUAGE: &str = "en";
const CACHE_CONTROL: &str = "private, immutable, max-age=31536000, s-maxage=0";

struct PreparedPut {
    body: Vec<u8>,
    content_sha256: String,
    headers: Vec<(String, String)>,
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the transport type lives in transport.rs and each operation stays beside its request building"
)]
impl S3Transport {
    pub async fn async_upload_data_to_s3<Payload>(
        &self,
        element: &S3BatchLoggingElement<Payload>,
    ) -> UploadOutcome
    where
        Payload: Serialize + Sync,
    {
        let prepared = match self.prepare_put(element) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::warn!(%error, key = %element.s3_object_key, "s3 logging: object could not be prepared");
                return UploadOutcome::Retry;
            }
        };
        let mut attempt = 0;
        loop {
            let response = self.put_attempt(element, &prepared).await;
            self.record_throttle(&response);
            let Ok(seen) = response else {
                tracing::warn!(key = %element.s3_object_key, "s3 logging: upload failed");
                return UploadOutcome::Retry;
            };
            let terminal = self.drop_on_terminal_error
                && is_terminal(seen.status, from_utf8(&seen.body).unwrap_or_default());
            if matches!(seen.status, 403 | 500 | 503) && !terminal && attempt < LAST_ATTEMPT {
                sleep(Duration::from_secs(1_u64 << attempt)).await;
                attempt = attempt.saturating_add(1);
                continue;
            }
            if (200..300).contains(&seen.status) {
                return UploadOutcome::Delivered;
            }
            tracing::warn!(
                status = seen.status,
                key = %element.s3_object_key,
                "s3 logging: S3 rejected the object"
            );
            return if terminal {
                UploadOutcome::Dropped
            } else {
                UploadOutcome::Retry
            };
        }
    }

    fn record_throttle(&self, response: &Result<S3Response, S3TransportError>) {
        let Some(sink) = self.throttle.as_ref() else {
            return;
        };
        let throttled = response.as_ref().map_or(true, |seen| {
            matches!(seen.status, 429 | 503)
                || error_code(from_utf8(&seen.body).unwrap_or_default()) == Some("SlowDown")
        });
        sink.record(throttled);
    }

    async fn put_attempt<Payload>(
        &self,
        element: &S3BatchLoggingElement<Payload>,
        prepared: &PreparedPut,
    ) -> Result<S3Response, S3TransportError>
    where
        Payload: Serialize + Sync,
    {
        let credentials = self.credentials.resolve().await?;
        let url = self.endpoint.object_url(&element.s3_object_key);
        let mut headers = prepared.headers.clone();
        let signed = sign_request(
            &RequestToSign {
                method: "PUT",
                url: &url,
                headers: &headers,
                content_sha256: &prepared.content_sha256,
                region: &self.endpoint.region,
                now: self.clock.now(),
            },
            &credentials,
        )?;
        headers.extend(signed);
        let request = S3Request {
            method: Method::PUT,
            url,
            headers,
            body: prepared.body.clone(),
        };
        self.client.send(request).await
    }

    fn prepare_put<Payload>(
        &self,
        element: &S3BatchLoggingElement<Payload>,
    ) -> Result<PreparedPut, S3TransportError>
    where
        Payload: Serialize,
    {
        let body = element_body(element)?;
        let content_sha256 = hex::encode(Sha256::digest(&body));
        let headers = put_headers(element, &body, &content_sha256, self.encryption.as_ref());
        Ok(PreparedPut {
            body,
            content_sha256,
            headers,
        })
    }
}

fn element_body<Payload>(
    element: &S3BatchLoggingElement<Payload>,
) -> Result<Vec<u8>, S3TransportError>
where
    Payload: Serialize,
{
    match &element.body {
        Some(body) => Ok(body.clone().into_bytes()),
        None => Ok(serde_json::to_vec(&element.payload)?),
    }
}

fn put_headers<Payload>(
    element: &S3BatchLoggingElement<Payload>,
    body: &[u8],
    content_sha256: &str,
    encryption: Option<&ServerSideEncryption>,
) -> Vec<(String, String)> {
    let mut headers = vec![
        (
            "Content-Type".to_owned(),
            content_type_name(element.content_type).to_owned(),
        ),
        ("Content-MD5".to_owned(), STANDARD.encode(Md5::digest(body))),
        ("x-amz-content-sha256".to_owned(), content_sha256.to_owned()),
        ("Content-Language".to_owned(), CONTENT_LANGUAGE.to_owned()),
        (
            "Content-Disposition".to_owned(),
            format!(
                "inline; filename=\"{}\"",
                element.s3_object_download_filename
            ),
        ),
        ("Cache-Control".to_owned(), CACHE_CONTROL.to_owned()),
    ];
    headers.extend(encryption_headers(encryption));
    headers
}

fn encryption_headers(encryption: Option<&ServerSideEncryption>) -> Vec<(String, String)> {
    let Some(configured) = encryption else {
        return Vec::new();
    };
    let candidates = [
        (
            "x-amz-server-side-encryption",
            configured.algorithm.as_str(),
        ),
        (
            "x-amz-server-side-encryption-aws-kms-key-id",
            configured.kms_key_id.as_deref().unwrap_or_default(),
        ),
    ];
    candidates
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

const fn content_type_name(content_type: ContentType) -> &'static str {
    match content_type {
        ContentType::Json => "application/json",
        ContentType::Ndjson => "application/x-ndjson",
    }
}

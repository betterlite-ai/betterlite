use betterlite_models::decisions::Json;
use http::Method;

use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::{RequestToSign, sign_request};
use crate::s3_transport::transport::{S3Request, S3Response, S3Transport};

mod tests;

pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

impl S3Transport {
    pub async fn download_object(&self, object_key: &str) -> Option<Json> {
        match self.fetch_object(object_key).await {
            Ok(object) => object,
            Err(error) => {
                tracing::warn!(%error, key = object_key, "s3 logging: object could not be downloaded");
                None
            }
        }
    }

    async fn fetch_object(&self, object_key: &str) -> Result<Option<Json>, S3TransportError> {
        let request = self.object_request(object_key).await?;
        let response = self.client.send(request).await?;
        Ok(parse_object(object_key, &response))
    }

    async fn object_request(&self, object_key: &str) -> Result<S3Request, S3TransportError> {
        let url = self.endpoint.object_url(object_key);
        let headers = vec![("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())];
        let credentials = self.credentials.resolve().await?;
        let signed = sign_request(
            &RequestToSign {
                method: "GET",
                url: &url,
                headers: &headers,
                content_sha256: EMPTY_SHA256,
                region: &self.endpoint.region,
                now: self.clock.now(),
            },
            &credentials,
        )?;
        let mut signed_headers = headers;
        signed_headers.extend(signed);
        Ok(S3Request {
            method: Method::GET,
            url,
            headers: signed_headers,
            body: Vec::new(),
        })
    }
}

fn parse_object(object_key: &str, response: &S3Response) -> Option<Json> {
    if response.status != 200 {
        tracing::warn!(
            status = response.status,
            key = object_key,
            "s3 logging: S3 did not return the object"
        );
        return None;
    }
    let Ok(object) = serde_json::from_slice::<Json>(&response.body) else {
        tracing::warn!(key = object_key, "s3 logging: object body is not JSON");
        return None;
    };
    let Json::Object(fields) = object else {
        tracing::warn!(
            key = object_key,
            "s3 logging: object body is not a JSON object"
        );
        return None;
    };
    Some(Json::Object(fields))
}

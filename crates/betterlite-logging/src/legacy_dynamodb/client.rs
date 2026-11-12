use std::error::Error as StdError;
use std::sync::Arc;

use async_trait::async_trait;
use aws_credential_types::Credentials;
use http::header::{HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use indexmap::IndexMap;
use serde::Serialize;
use url::Url;

use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};
use crate::sqs_logging::clock::Clock;
use crate::sqs_logging::credentials::{CredentialError, CredentialParams, CredentialSource};

use super::signing::{self, Signable, SigningError};

mod tests;

const CONTENT_TYPE: &str = "application/x-amz-json-1.0";
const TARGET: &str = "DynamoDB_20120810.PutItem";
// boto3 raises for a 4xx or 5xx answer and returns the parsed body otherwise
const FIRST_ERROR_STATUS: u16 = 400;

#[async_trait]
pub trait DynamoClient: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn put_item(
        &self,
        table: &str,
        item: &IndexMap<String, String>,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum PutItemError<E>
where
    E: StdError + 'static,
{
    #[error("the DynamoDB credentials could not be resolved")]
    Credentials(#[source] CredentialError),
    #[error("the DynamoDB item could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the DynamoDB endpoint URL could not be parsed")]
    Url(#[source] url::ParseError),
    #[error("the DynamoDB request header name {name} is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the DynamoDB request header value of {name} is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the DynamoDB request could not be signed")]
    Signing(#[from] SigningError),
    #[error("the DynamoDB delivery failed")]
    Delivery(#[source] E),
    #[error("DynamoDB answered with status {status}")]
    Status { status: u16 },
}

#[derive(Serialize)]
struct PutItemBody<'item> {
    #[serde(rename = "TableName")]
    table_name: &'item str,
    #[serde(rename = "Item")]
    item: IndexMap<String, AttributeValue<'item>>,
}

#[derive(Serialize)]
struct AttributeValue<'text> {
    #[serde(rename = "S")]
    string: &'text str,
}

pub struct HttpDynamoClient<D> {
    pub delivery: D,
    pub credentials: Arc<dyn CredentialSource>,
    pub params: CredentialParams,
    pub clock: Arc<dyn Clock>,
    pub endpoint: String,
    pub region: String,
}

impl<D: Delivery> HttpDynamoClient<D> {
    async fn send_put_item(
        &self,
        table: &str,
        item: &IndexMap<String, String>,
    ) -> Result<(), PutItemError<D::Error>> {
        let credentials = self
            .credentials
            .resolve(&self.params)
            .await
            .map_err(PutItemError::Credentials)?;
        let body = encode_item(table, item)?;
        let url = Url::parse(&self.endpoint).map_err(PutItemError::Url)?;
        let headers = self.request_headers(&url, &body, &credentials)?;
        let response = self
            .delivery
            .post(PostRequest {
                url,
                headers,
                body,
                deadline: DELIVERY_DEADLINE,
            })
            .await
            .map_err(PutItemError::Delivery)?;
        if response.status >= FIRST_ERROR_STATUS {
            return Err(PutItemError::Status {
                status: response.status,
            });
        }
        Ok(())
    }

    fn request_headers(
        &self,
        url: &Url,
        body: &[u8],
        credentials: &Credentials,
    ) -> Result<HeaderMap, PutItemError<D::Error>> {
        let unsigned = [
            ("content-type".to_owned(), CONTENT_TYPE.to_owned()),
            ("x-amz-target".to_owned(), TARGET.to_owned()),
        ];
        let signed = signing::signed_headers(
            &Signable {
                method: "POST",
                url: url.as_str(),
                headers: &unsigned,
                body,
                region: &self.region,
                now: self.clock.now(),
            },
            credentials,
        )?;
        header_map(&unsigned, &signed)
    }
}

fn encode_item<E>(table: &str, item: &IndexMap<String, String>) -> Result<Vec<u8>, PutItemError<E>>
where
    E: StdError + 'static,
{
    let attributes: IndexMap<String, AttributeValue<'_>> = item
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                AttributeValue {
                    string: value.as_str(),
                },
            )
        })
        .collect();
    serde_json::to_vec(&PutItemBody {
        table_name: table,
        item: attributes,
    })
    .map_err(PutItemError::Encode)
}

fn header_map<E>(
    unsigned: &[(String, String)],
    signed: &[(String, String)],
) -> Result<HeaderMap, PutItemError<E>>
where
    E: StdError + 'static,
{
    let mut headers = HeaderMap::new();
    for (name, value) in unsigned.iter().chain(signed) {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| PutItemError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| PutItemError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

#[async_trait]
impl<D: Delivery> DynamoClient for HttpDynamoClient<D> {
    type Error = PutItemError<D::Error>;

    async fn put_item(
        &self,
        table: &str,
        item: &IndexMap<String, String>,
    ) -> Result<(), Self::Error> {
        self.send_put_item(table, item).await
    }
}

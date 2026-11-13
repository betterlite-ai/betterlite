use std::str::from_utf8;
use std::sync::Arc;

use aws_credential_types::Credentials;
use http::header::CONTENT_TYPE;
use http::{Method, Request};
use url::Url;

use crate::fields::payload::StandardLoggingPayload;
use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};

use super::clock::Clock;
use super::credentials::{CredentialError, CredentialSource};
use super::health::HealthPayloadFactory;
use super::message::{FORM_CONTENT_TYPE, MessageError, form_body, message_json};
use super::settings::Settings;
use super::signing::{SigningError, sign_request};

// httpx's raise_for_status only raises for 4xx and 5xx, so a redirect counts as a delivered message
const FIRST_ERROR_STATUS: u16 = 400;

#[derive(Debug, thiserror::Error)]
pub enum SendError<E> {
    #[error("sqs_queue_url not set")]
    MissingQueueUrl,
    #[error("sqs_region_name not set")]
    MissingRegion,
    #[error("the SQS queue URL could not be parsed")]
    QueueUrl(#[source] url::ParseError),
    #[error("the SQS credentials could not be resolved")]
    Credentials(#[source] CredentialError),
    #[error("the SQS message could not be built")]
    Message(#[source] MessageError),
    #[error("the SQS request could not be built")]
    Request(#[source] http::Error),
    #[error("the SQS request could not be signed")]
    Signing(#[source] SigningError),
    #[error("the SQS delivery failed")]
    Transport(#[source] E),
    #[error("SQS answered with status {status}")]
    Status { status: u16, detail: String },
}

pub struct SqsSender<D> {
    pub(crate) settings: Settings,
    pub(crate) delivery: D,
    pub(crate) credentials: Arc<dyn CredentialSource>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) health_payload: Arc<dyn HealthPayloadFactory>,
}

impl<D> SqsSender<D>
where
    D: Delivery,
{
    // LiteLLM catches every SQS send failure and logs it, so a callback never blocks the request
    pub(crate) async fn async_send_message(&self, payload: &StandardLoggingPayload) {
        if let Err(error) = self.send_message(payload).await {
            tracing::error!(%error, "Error sending to SQS");
        }
    }

    async fn send_message(
        &self,
        payload: &StandardLoggingPayload,
    ) -> Result<(), SendError<D::Error>> {
        let credentials = self
            .credentials
            .resolve(&self.settings.credentials)
            .await
            .map_err(SendError::Credentials)?;
        let post = build_post(self, payload, &credentials)?;
        let response = self
            .delivery
            .post(post)
            .await
            .map_err(SendError::Transport)?;
        if response.status >= FIRST_ERROR_STATUS {
            let detail = from_utf8(&response.body)
                .map_or_else(|_error| "<non-UTF-8 body>".to_owned(), String::from);
            return Err(SendError::Status {
                status: response.status,
                detail,
            });
        }
        Ok(())
    }
}

fn build_post<D>(
    sender: &SqsSender<D>,
    payload: &StandardLoggingPayload,
    credentials: &Credentials,
) -> Result<PostRequest, SendError<D::Error>>
where
    D: Delivery,
{
    let queue_url = sender
        .settings
        .connection
        .queue_url
        .as_deref()
        .ok_or(SendError::MissingQueueUrl)?;
    let url = Url::parse(queue_url).map_err(SendError::QueueUrl)?;
    let region = sender
        .settings
        .connection
        .region
        .as_deref()
        .ok_or(SendError::MissingRegion)?;
    let json =
        message_json(payload, sender.settings.encryption.as_ref()).map_err(SendError::Message)?;
    let body = form_body(&json).into_bytes();
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(url.as_str())
        .header(CONTENT_TYPE, FORM_CONTENT_TYPE)
        .body(body)
        .map_err(SendError::Request)?;
    sign_request(&mut request, credentials, region, sender.clock.now())
        .map_err(SendError::Signing)?;
    Ok(PostRequest {
        url,
        headers: request.headers().clone(),
        body: request.into_body(),
        deadline: DELIVERY_DEADLINE,
    })
}

mod tests;

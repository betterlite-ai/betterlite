use std::time::Duration;

use betterlite_http::url_policy::origin::assert_same_origin;
use http::Method;
use http::header::HeaderMap;

use crate::image_transport::body;
use crate::image_transport::decode::ImageOperation;
use crate::image_transport::error::ImageTransportError;
use crate::image_transport::json::{JsonMap, JsonValue};
use crate::image_transport::request::poll::Poller;
use crate::image_transport::{
    ImageClock, ImageReply, ImageRequest, ImageResponse, ImageTransport, RequestMetadata,
};

mod poll;

const SUBMIT_PATH: &str = "/openai/images/generations:submit";
const SYNTHETIC_REQUEST_URL: &str = "https://api.openai.com/v1";
const OPERATION_LOCATION: &str = "operation-location";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutPhases {
    pub connect: f64,
    pub read: f64,
    pub write: f64,
    pub pool: f64,
}

impl TimeoutPhases {
    const DEFAULT: Self = Self {
        connect: 5.0_f64,
        read: 600.0_f64,
        write: 600.0_f64,
        pool: 600.0_f64,
    };

    #[must_use]
    pub const fn from_argument(timeout: Option<f64>) -> Self {
        match timeout {
            None => Self::DEFAULT,
            Some(seconds) => Self {
                connect: seconds,
                read: seconds,
                write: seconds,
                pool: seconds,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStyle {
    Async,
    Sync,
}

#[derive(Debug, Clone, Copy)]
pub struct PollingSettings {
    pub timeout: Duration,
    pub style: CallStyle,
}

pub struct Connection<Client> {
    pub client: Option<Client>,
    pub timeout: Option<f64>,
}

pub trait ImageClientConstructor: Sync {
    type Client: ImageTransport;

    fn construct(&self, timeout: TimeoutPhases) -> Self::Client;
}

pub struct Call<'data> {
    pub api_base: &'data str,
    pub api_version: &'data str,
    pub deployment_name: Option<&'data str>,
    pub data: &'data mut JsonMap,
    pub headers: HeaderMap,
}

pub async fn execute<C, K>(
    constructor: &C,
    connection: Connection<C::Client>,
    clock: &K,
    settings: PollingSettings,
    call: Call<'_>,
) -> Result<ImageResponse, ImageTransportError>
where
    C: ImageClientConstructor,
    K: ImageClock,
{
    let client = connection
        .client
        .unwrap_or_else(|| constructor.construct(TimeoutPhases::from_argument(connection.timeout)));
    if call.api_base.contains("images/generations")
        && body::PREVIEW_API_VERSIONS.contains(&call.api_version)
    {
        return poll_operation(&client, clock, settings, call).await;
    }
    send_direct(&client, call).await
}

async fn send_direct<T>(client: &T, call: Call<'_>) -> Result<ImageResponse, ImageTransportError>
where
    T: ImageTransport,
{
    let Call {
        api_base,
        deployment_name,
        data,
        headers,
        ..
    } = call;
    let body = body::image_generation_body(api_base, data, deployment_name);
    let payload = serde_json::to_vec(&body)?;
    let reply = send(client, Method::POST, api_base, &headers, payload).await?;
    Ok(ImageResponse {
        status: reply.status,
        headers: reply.headers,
        body: reply.body,
        request: RequestMetadata {
            method: Method::POST,
            url: api_base.to_owned(),
        },
    })
}

async fn poll_operation<T, K>(
    client: &T,
    clock: &K,
    settings: PollingSettings,
    call: Call<'_>,
) -> Result<ImageResponse, ImageTransportError>
where
    T: ImageTransport,
    K: ImageClock,
{
    let Call {
        api_base,
        data,
        headers,
        ..
    } = call;
    let submit_url = body::modify_url(api_base, SUBMIT_PATH)?;
    data.shift_remove("model");
    let payload = serde_json::to_vec(&*data)?;
    let submitted = send(client, Method::POST, &submit_url, &headers, payload).await?;
    let location = operation_location(&submitted)?;
    assert_same_origin(&location, &submit_url)
        .map_err(|source| ImageTransportError::RejectedPollingUrl { source })?;
    let first = send(client, Method::GET, &location, &headers, Vec::new()).await?;
    let poller = Poller {
        client,
        clock,
        settings,
        location: &location,
        headers: &headers,
    };
    let (reply, operation) = poller.run(first, clock.now()).await?;
    finish(reply, operation)
}

pub(super) async fn send<T>(
    client: &T,
    method: Method,
    url: &str,
    headers: &HeaderMap,
    body: Vec<u8>,
) -> Result<ImageReply, ImageTransportError>
where
    T: ImageTransport,
{
    let request = ImageRequest::new(method, url.to_owned(), headers.clone(), body);
    client
        .send(request)
        .await
        .map_err(ImageTransportError::Transport)
}

fn finish(
    reply: ImageReply,
    operation: ImageOperation,
) -> Result<ImageResponse, ImageTransportError> {
    if matches!(operation.status.as_ref(), Some(JsonValue::String(status)) if status.as_str() == "failed")
    {
        return Err(ImageTransportError::operation_failed(operation));
    }
    let result = operation.result.ok_or(ImageTransportError::MissingResult)?;
    let body = serde_json::to_vec(&result)?;
    Ok(ImageResponse {
        status: 200,
        headers: reply.headers,
        body,
        request: RequestMetadata {
            method: Method::POST,
            url: SYNTHETIC_REQUEST_URL.to_owned(),
        },
    })
}

fn operation_location(reply: &ImageReply) -> Result<String, ImageTransportError> {
    reply
        .headers
        .get(OPERATION_LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| ImageTransportError::MissingOperationLocation {
            body: response_text(&reply.body),
        })
}

fn response_text(body: &[u8]) -> String {
    String::from_utf8(body.to_vec()).unwrap_or_else(|error| {
        let length = error.as_bytes().len();
        format!("{length} bytes that are not UTF-8")
    })
}

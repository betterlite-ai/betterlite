use std::time::Duration;

use http::Method;
use http::header::HeaderMap;

use crate::image_transport::decode::{ImageOperation, decode_image_operation};
use crate::image_transport::error::ImageTransportError;
use crate::image_transport::json::JsonValue;
use crate::image_transport::{ImageClock, ImageReply, ImageTransport};

use super::{CallStyle, PollingSettings, send};

const RETRY_AFTER: &str = "retry-after";
const DEFAULT_RETRY_AFTER_SECONDS: i64 = 10;

pub(super) struct Poller<'call, T, K> {
    pub(super) client: &'call T,
    pub(super) clock: &'call K,
    pub(super) settings: PollingSettings,
    pub(super) location: &'call str,
    pub(super) headers: &'call HeaderMap,
}

impl<T, K> Poller<'_, T, K>
where
    T: ImageTransport,
    K: ImageClock,
{
    pub(super) async fn run(
        &self,
        first: ImageReply,
        started: Duration,
    ) -> Result<(ImageReply, ImageOperation), ImageTransportError> {
        let mut reply = first;
        let mut operation = decode_image_operation(&reply.body)
            .map_err(|source| ImageTransportError::Decode { source })?;
        let mut status = operation
            .status
            .clone()
            .ok_or(ImageTransportError::MissingStatusInitial)?;
        while !matches!(&status, JsonValue::String(value) if value.as_str() == "succeeded" || value.as_str() == "failed")
        {
            if self.clock.now().saturating_sub(started) > self.settings.timeout {
                return Err(ImageTransportError::PollingTimeout);
            }
            let seconds = retry_after(&reply.headers)?;
            sleep(self.clock, seconds, self.settings.style).await?;
            reply = send(
                self.client,
                Method::GET,
                self.location,
                self.headers,
                Vec::new(),
            )
            .await?;
            operation = decode_image_operation(&reply.body)
                .map_err(|source| ImageTransportError::Decode { source })?;
            status = operation
                .status
                .clone()
                .ok_or(ImageTransportError::MissingStatus)?;
        }
        Ok((reply, operation))
    }
}

fn retry_after(headers: &HeaderMap) -> Result<i64, ImageTransportError> {
    let Some(raw) = headers.get(RETRY_AFTER) else {
        return Ok(DEFAULT_RETRY_AFTER_SECONDS);
    };
    let Ok(value) = raw.to_str() else {
        return Err(ImageTransportError::RetryAfter {
            value: format!("{raw:?}"),
        });
    };
    if value.is_empty() {
        return Ok(DEFAULT_RETRY_AFTER_SECONDS);
    }
    value
        .trim()
        .parse::<i64>()
        .map_err(|_source| ImageTransportError::RetryAfter {
            value: value.to_owned(),
        })
}

async fn sleep<K>(clock: &K, seconds: i64, style: CallStyle) -> Result<(), ImageTransportError>
where
    K: ImageClock,
{
    if style == CallStyle::Sync && seconds < 0 {
        return Err(ImageTransportError::NegativeSleep { seconds });
    }
    clock.pause(seconds).await;
    Ok(())
}

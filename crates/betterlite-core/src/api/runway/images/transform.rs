use betterlite_openai::api::image_contracts::records::{
    ImageHeaders, ImageHttpResponse, ImageResponse,
};
use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};

use super::error::RunwayImageError;
use super::reply::apply_task_output;
use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;
use crate::api::runway::tasks::poll::{PollClock, TaskPoller, TaskTransport};
use crate::api::runway::tasks::record::decode_runway_task;
use crate::api::runway::tasks::settings::RunwaySettings;

const POLL_API_BASE: &str = "https://api.dev.runwayml.com";
const VERSION_HEADER: &str = "x-runway-version";

pub struct RunwayImageTransform<'client, T, C> {
    pub transport: &'client T,
    pub clock: &'client C,
    pub settings: &'client RunwaySettings,
}

impl<T: TaskTransport + Sync, C: PollClock + Sync> RunwayImageTransform<'_, T, C> {
    pub async fn transform(
        &self,
        submission: &ImageHttpResponse,
        request_headers: &ImageHeaders,
        response: ImageResponse,
    ) -> Result<ImageResponse, RunwayImageError> {
        let task = decode_runway_task(&submission.body).map_err(|source| {
            RunwayImageError::Submission {
                status: submission.status_code,
                headers: submission.headers.clone(),
                source,
            }
        })?;
        let task_id = task
            .id
            .filter(|id| !id.is_empty())
            .ok_or(RunwayImageError::MissingTaskId)?;
        let headers = poll_headers(request_headers, &self.settings.api_version)?;
        let poller = TaskPoller {
            transport: self.transport,
            clock: self.clock,
            kind: TaskKind::Image,
            timeout_seconds: self.settings.polling_timeout_seconds,
        };
        let completed = poller.poll(POLL_API_BASE, &task_id, &headers).await?;
        Ok(apply_task_output(&completed, response))
    }
}

fn poll_headers(
    original: &ImageHeaders,
    default_version: &str,
) -> Result<HeaderMap, RunwayImageError> {
    let authorization = header_value(original, "authorization").unwrap_or_default();
    let version = header_value(original, VERSION_HEADER).unwrap_or(default_version);
    let mut bearer =
        HeaderValue::try_from(authorization).map_err(RunwayTaskError::InvalidHeader)?;
    bearer.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, bearer);
    headers.insert(
        HeaderName::from_static(VERSION_HEADER),
        HeaderValue::try_from(version).map_err(RunwayTaskError::InvalidHeader)?,
    );
    Ok(headers)
}

fn header_value<'headers>(headers: &'headers ImageHeaders, name: &str) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

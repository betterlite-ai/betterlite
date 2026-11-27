use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::files::stream::flatten_headers;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use url::Url;

use super::error::RunwaySpeechError;
use crate::api::api_boundaries::ApiLimits;
use crate::api::runway::tasks::TaskKind;
use crate::api::runway::tasks::error::RunwayTaskError;
use crate::api::runway::tasks::poll::{PollClock, TaskPoller, TaskTransport};
use crate::api::runway::tasks::record::{TaskOutput, TaskOutputs, decode_runway_task};
use crate::api::runway::tasks::settings::RunwaySettings;
use crate::api::runway::video_lifecycle::artifact::ArtifactTransport;

const POLL_API_BASE: &str = "https://api.dev.runwayml.com";
const VERSION_HEADER: &str = "x-runway-version";

pub struct RunwaySpeechTransform<'client, T, C> {
    pub transport: &'client T,
    pub clock: &'client C,
    pub settings: &'client RunwaySettings,
    pub limits: ApiLimits,
    pub deadline: Duration,
}

impl<T: TaskTransport + ArtifactTransport, C: PollClock + Sync> RunwaySpeechTransform<'_, T, C> {
    pub async fn transform(
        &self,
        submission: &UpstreamResponse,
        request_headers: &IndexMap<String, String>,
    ) -> Result<BinaryContent, RunwaySpeechError> {
        let task = decode_runway_task(&submission.body).map_err(|source| {
            RunwaySpeechError::Submission {
                status: submission.status,
                headers: flatten_headers(&submission.headers),
                source: Box::new(source),
            }
        })?;
        let task_id = task
            .id
            .filter(|id| !id.is_empty())
            .ok_or(RunwaySpeechError::MissingTaskId)?;
        let headers = poll_headers(request_headers, &self.settings.api_version)?;
        let completed = TaskPoller {
            transport: self.transport,
            clock: self.clock,
            kind: TaskKind::Speech,
            timeout_seconds: self.settings.polling_timeout_seconds,
        }
        .poll(POLL_API_BASE, &task_id, &headers)
        .await?;
        let audio = audio_url(completed.output.as_ref())?;
        self.download(audio).await
    }

    async fn download(&self, audio: &str) -> Result<BinaryContent, RunwaySpeechError> {
        let url = Url::parse(audio).map_err(RunwaySpeechError::AudioUrl)?;
        let request = OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: self.deadline,
        };
        let reply = self
            .transport
            .fetch(request, self.limits.max_json_bytes)
            .await
            .map_err(RunwaySpeechError::Download)?;
        if !(200..300).contains(&reply.status) {
            return Err(RunwaySpeechError::DownloadStatus {
                status: reply.status,
            });
        }
        Ok(BinaryContent::from_response(reply))
    }
}

// the source builds the poll headers from the submission request's own Authorization and version
fn poll_headers(
    original: &IndexMap<String, String>,
    default_version: &str,
) -> Result<HeaderMap, RunwaySpeechError> {
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

fn header_value<'headers>(
    headers: &'headers IndexMap<String, String>,
    name: &str,
) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

// the first output only: a later string never replaces a first member the source cannot use
fn audio_url(output: Option<&TaskOutputs>) -> Result<&str, RunwaySpeechError> {
    let Some(TaskOutputs::Members(members)) = output else {
        return Err(RunwaySpeechError::MissingAudioUrl);
    };
    let Some(first) = members.first() else {
        return Err(RunwaySpeechError::MissingAudioUrl);
    };
    match first {
        TaskOutput::Url(url) => Ok(url),
        TaskOutput::Fields(_) | TaskOutput::Unsupported(_) => {
            Err(RunwaySpeechError::AudioUrlNotString)
        }
    }
}

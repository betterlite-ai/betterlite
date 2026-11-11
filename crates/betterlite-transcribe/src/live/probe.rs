use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt as _;
use rust_decimal::Decimal;
use tokio::fs::File;
use tokio::io::{AsyncSeekExt as _, AsyncWriteExt as _};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::live::exchange::{BodyStream, HttpMethod, HttpTransport, OutboundHttp};
use crate::live::signing::AwsSigner;
use crate::live::{ProbeError, S3MediaProbe};
use crate::media_pricing::MediaDurationProbe;
use crate::media_source::{
    MediaSizeGuard, content_length_within_limit, media_duration_seconds, media_predates_job,
    s3_media_url,
};

// B173's client has no total deadline, so the media download gets one of its own
const MEDIA_DEADLINE: Duration = Duration::from_hours(2);

impl<S, H> S3MediaProbe<S, H> {
    pub fn new(region: &str, signer: S, http: H, download_slots: Arc<Semaphore>) -> Self {
        Self {
            region: region.to_owned(),
            signer,
            http,
            download_slots,
        }
    }
}

#[async_trait]
impl<S, H> MediaDurationProbe for S3MediaProbe<S, H>
where
    S: AwsSigner,
    H: HttpTransport,
{
    async fn media_seconds(
        &self,
        media_uri: &str,
        job_created_at: f64,
    ) -> Result<Option<Decimal>, ProbeError> {
        let Some(url) = s3_media_url(media_uri, &self.region)? else {
            return Ok(None);
        };
        let signed = self.signer.sign_s3_get(&self.region, &url).await?;
        // one download at a time per B3646's semaphore; held until the length is read
        let _slot = self
            .download_slots
            .acquire()
            .await
            .map_err(ProbeError::Slots)?;
        let answer = self
            .http
            .send(OutboundHttp {
                method: HttpMethod::Get,
                url: signed.url,
                headers: signed.headers,
                body: signed.body,
                deadline: MEDIA_DEADLINE,
            })
            .await?;
        if !(200..300).contains(&answer.status) {
            return Err(ProbeError::Status {
                url: Box::from(url.as_str()),
                status: answer.status,
            });
        }
        if !media_predates_job(&answer.headers, job_created_at) {
            tracing::warn!(
                media_uri = %media_uri,
                "Transcribe media was rewritten after the job was created, charging maximum"
            );
            return Ok(None);
        }
        if !content_length_within_limit(&answer.headers)? {
            tracing::warn!(
                media_uri = %media_uri,
                "Transcribe media exceeds the size cap, charging maximum"
            );
            return Ok(None);
        }
        download_and_measure(answer.body, media_uri, MediaSizeGuard::default()).await
    }
}

pub(super) async fn download_and_measure(
    mut body: BodyStream,
    media_uri: &str,
    mut guard: MediaSizeGuard,
) -> Result<Option<Decimal>, ProbeError> {
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(tempfile::tempfile);
    let Some(created) = tasks.join_next().await else {
        return Err(ProbeError::TaskClosed);
    };
    let mut file = File::from_std(created.map_err(ProbeError::Task)?.map_err(ProbeError::Io)?);
    while let Some(result) = body.next().await {
        let chunk = result?;
        if !guard.accept(chunk.len()) {
            tracing::warn!(
                media_uri = %media_uri,
                "Transcribe media exceeds the size cap, charging maximum"
            );
            return Ok(None);
        }
        file.write_all(&chunk).await.map_err(ProbeError::Io)?;
    }
    file.rewind().await.map_err(ProbeError::Io)?;
    let media = file.into_std().await;
    let mut decode = JoinSet::new();
    decode.spawn_blocking(move || media_duration_seconds(media));
    let Some(measured) = decode.join_next().await else {
        return Err(ProbeError::TaskClosed);
    };
    measured.map_err(ProbeError::Task)
}

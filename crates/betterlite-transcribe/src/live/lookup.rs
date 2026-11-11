use std::time::Duration;

use betterlite_context::aws_partition::get_aws_partition;
use futures_util::StreamExt as _;

use crate::live::TranscribeJobLookup;
use crate::live::exchange::{BodyStream, HttpMethod, HttpTransport, OutboundHttp};
use crate::live::signing::AwsSigner;
use crate::polling::aws_error_type;
use crate::records::{GetTranscriptionJobRequest, JobLookup, JobLookupError, JobLookupFuture};

pub const MAX_LOOKUP_BODY_BYTES: usize = 0x0010_0000;

const CONTENT_TYPE: &str = "application/x-amz-json-1.1";
const TARGET: &str = "Transcribe.GetTranscriptionJob";
const LOOKUP_DEADLINE: Duration = Duration::from_secs(600);

impl<S, H> TranscribeJobLookup<S, H> {
    pub fn new(region: &str, signer: S, http: H) -> Self {
        let dns_suffix = get_aws_partition(Some(region)).dns_suffix;
        Self {
            region: region.to_owned(),
            endpoint: format!("https://transcribe.{region}.{dns_suffix}/"),
            signer,
            http,
        }
    }
}

impl<S, H> TranscribeJobLookup<S, H>
where
    S: AwsSigner,
    H: HttpTransport,
{
    async fn fetch(
        &self,
        request: &GetTranscriptionJobRequest,
    ) -> Result<(u16, Vec<u8>), JobLookupError> {
        let body = serde_json::to_string(request).map_err(|_error| JobLookupError::Unavailable)?;
        let headers = [
            ("Content-Type".to_owned(), CONTENT_TYPE.to_owned()),
            ("X-Amz-Target".to_owned(), TARGET.to_owned()),
        ];
        let signed = self
            .signer
            .sign_json_post(&self.region, &self.endpoint, &body, &headers)
            .await
            .map_err(|_error| JobLookupError::Unavailable)?;
        let answer = self
            .http
            .send(OutboundHttp {
                method: HttpMethod::Post,
                url: signed.url,
                headers: signed.headers,
                body: signed.body,
                deadline: LOOKUP_DEADLINE,
            })
            .await
            .map_err(|_error| JobLookupError::Unavailable)?;
        let status = answer.status;
        let Some(payload) = read_body(answer.body, MAX_LOOKUP_BODY_BYTES).await else {
            return Err(JobLookupError::Unavailable);
        };
        Ok((status, payload))
    }
}

impl<S, H> JobLookup for TranscribeJobLookup<S, H>
where
    S: AwsSigner,
    H: HttpTransport,
{
    fn get_transcription_job<'lookup>(
        &'lookup self,
        request: &'lookup GetTranscriptionJobRequest,
    ) -> JobLookupFuture<'lookup> {
        Box::pin(async move {
            let (status, payload) = self.fetch(request).await?;
            if !(200..300).contains(&status) {
                return Err(JobLookupError::Rejected {
                    status,
                    aws_error_type: aws_error_type(&payload),
                });
            }
            serde_json::from_slice(&payload).map_err(JobLookupError::InvalidResponse)
        })
    }
}

async fn read_body(mut body: BodyStream, limit: usize) -> Option<Vec<u8>> {
    let mut payload = Vec::new();
    while let Some(result) = body.next().await {
        let chunk = result.ok()?;
        let total = payload.len().checked_add(chunk.len())?;
        if total > limit {
            return None;
        }
        payload.extend_from_slice(&chunk);
    }
    Some(payload)
}

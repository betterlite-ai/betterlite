use std::io::Error as IoError;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_bedrock::credentials::AwsCredentialResolver;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::error::SigningError;
use betterlite_http::error::HttpError as BodyError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::sync::{AcquireError, Semaphore};
use tokio::task::JoinError;

use crate::live::exchange::PooledHttp;
use crate::media_pricing::{JobPricing, MediaDurationProbe, PricingError, price_transcription_job};
use crate::media_source::MediaSourceError;
use crate::policy::cost::transcribe_max_job_cost;
use crate::polling::{MAX_POLLING_ATTEMPTS, PollSleeper, TokioSleeper};
use crate::records::{JobLookup, StartedJob};

pub mod exchange;
pub mod lookup;
pub mod probe;
pub mod signing;
mod tests;

pub struct TranscribeJobLookup<S, H> {
    region: String,
    endpoint: String,
    signer: S,
    http: H,
}

pub struct S3MediaProbe<S, H> {
    region: String,
    signer: S,
    http: H,
    download_slots: Arc<Semaphore>,
}

pub struct LiveDeps<'deps> {
    pub resolver: &'deps AwsCredentialResolver<'deps>,
    pub http: ProxiedClient,
    pub region: String,
    pub download_slots: Arc<Semaphore>,
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("the media URL cannot be used")]
    Source(#[from] MediaSourceError),
    #[error("the AWS credentials could not be resolved")]
    Credentials(#[source] Box<CredentialError>),
    #[error("the AWS request could not be signed")]
    Signing(#[source] SigningError),
    #[error("the request URL is not valid")]
    Url(#[source] url::ParseError),
    #[error("the {name} header name is not valid")]
    HeaderName {
        name: Box<str>,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not valid")]
    HeaderValue {
        name: Box<str>,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the request could not be sent")]
    Transport(#[source] TransportError),
    #[error("the response body failed")]
    Body(#[source] BodyError),
    #[error("the request to {url} was answered with status {status}")]
    Status { url: Box<str>, status: u16 },
    #[error("the media could not be stored for decoding")]
    Io(#[source] IoError),
    #[error("the download slot is no longer open")]
    Slots(#[source] AcquireError),
    #[error("the media decoding task failed")]
    Task(#[source] JoinError),
    #[error("the media decoding task set closed without a result")]
    TaskClosed,
}

// it only wires the real adapters, and price_fail_closed carries every decision it makes
#[cfg_attr(test, mutants::skip)]
pub async fn price_transcription_job_live(
    job_name: &str,
    cost_per_second: Decimal,
    started_job: StartedJob,
    deps: LiveDeps<'_>,
) -> Result<Decimal, PricingError> {
    let signer = signing::ClockedSigner {
        credentials: signing::ResolverCredentials(deps.resolver),
        clock: SystemTime::now,
    };
    let http = PooledHttp { client: deps.http };
    let lookup = TranscribeJobLookup::new(&deps.region, signer, http.clone());
    let probe = S3MediaProbe::new(&deps.region, signer, http, deps.download_slots);
    let pricing = JobPricing {
        job_name,
        cost_per_second,
        max_attempts: MAX_POLLING_ATTEMPTS,
        started_job,
    };
    price_fail_closed(pricing, &lookup, &probe, &TokioSleeper).await
}

pub async fn price_fail_closed<L, P, S>(
    pricing: JobPricing<'_>,
    lookup: &L,
    probe: &P,
    sleeper: &S,
) -> Result<Decimal, PricingError>
where
    L: JobLookup + ?Sized,
    P: MediaDurationProbe + ?Sized,
    S: PollSleeper + ?Sized,
{
    let job_name = pricing.job_name;
    let cost_per_second = pricing.cost_per_second;
    match price_transcription_job(pricing, lookup, probe, sleeper).await {
        Ok(cost) => Ok(cost),
        Err(error) => {
            tracing::error!(
                job_name = %job_name,
                error = %error,
                "Pricing Transcribe job failed, charging maximum"
            );
            transcribe_max_job_cost(cost_per_second).map_err(PricingError::from)
        }
    }
}

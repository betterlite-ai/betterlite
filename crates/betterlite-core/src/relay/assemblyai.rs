use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::split::urlparse;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;
use url::Url;

use crate::api::media_relay_logging::spend::MediaSpend;

pub mod poll;
mod pricing;
mod tests;
mod transcript;

pub const ASSEMBLY_AI_PROVIDER: &str = "assemblyai";
pub const ASSEMBLY_AI_FALLBACK_MODEL: &str = "assemblyai/nano";

const ASSEMBLY_AI_BASE_URL: &str = "https://api.assemblyai.com";
const ASSEMBLY_AI_EU_BASE_URL: &str = "https://eu.assemblyai.com";
const ROUTING_BASE_URL: &str = "https://api.assemblyai.com";
const ROUTING_EU_BASE_URL: &str = "https://api.eu.assemblyai.com";
const ASSEMBLY_AI_POLLING_INTERVAL: Duration = Duration::from_secs(10);
const MAX_POLLING_ATTEMPTS: u32 = 180;
const EU_REGION: &str = "eu";
const EU_HOST: &str = "eu.assemblyai.com";

// The handler's __init__: both polling base URLs and the poll cadence
#[derive(Clone, Copy, Debug)]
pub struct AssemblyAiSettings {
    pub polling_base_url: &'static str,
    pub polling_eu_base_url: &'static str,
    pub polling_interval: Duration,
    pub max_polling_attempts: u32,
}

impl AssemblyAiSettings {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            polling_base_url: ASSEMBLY_AI_BASE_URL,
            polling_eu_base_url: ASSEMBLY_AI_EU_BASE_URL,
            polling_interval: ASSEMBLY_AI_POLLING_INTERVAL,
            max_polling_attempts: MAX_POLLING_ATTEMPTS,
        }
    }

    // _get_assembly_transcript picks its base by region and leaves every other region on the default
    #[must_use]
    pub fn polling_base(&self, region: Option<&str>) -> &'static str {
        if region == Some(EU_REGION) {
            self.polling_eu_base_url
        } else {
            self.polling_base_url
        }
    }
}

impl Default for AssemblyAiSettings {
    fn default() -> Self {
        Self::new()
    }
}

// _get_assembly_region_from_url, for the relay route and the logging handler alike
#[must_use]
pub fn assembly_region_from_url(url: Option<&str>) -> Option<&'static str> {
    if urlparse(url?, "")
        .ok()?
        .hostname()
        .is_some_and(|host| host == EU_HOST)
    {
        return Some(EU_REGION);
    }
    None
}

// _get_assembly_base_url_from_region: the routing base, which differs from the EU polling base
#[must_use]
pub fn assembly_base_url_from_region(region: Option<&str>) -> &'static str {
    if region == Some(EU_REGION) {
        ROUTING_EU_BASE_URL
    } else {
        ROUTING_BASE_URL
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssemblyAiError {
    #[error("Transcript ID is required to log the cost of the transcription")]
    MissingTranscriptId,
    #[error("Invalid transcript_id '{transcript_id}': contains disallowed characters")]
    InvalidTranscriptId { transcript_id: String },
    #[error("AssemblyAI API key not found")]
    MissingApiKey,
    #[error("the {field} value must be a string")]
    TextField { field: &'static str },
    #[error("the {field} value must be a number")]
    NumberField { field: &'static str },
    #[error("the AssemblyAI transcript answer is not a JSON object")]
    TranscriptShape,
    #[error("the transcript cost is out of range")]
    CostOverflow,
}

// PassthroughEndpointRouter.get_credentials: the provider's key, optionally for one region
pub trait AssemblyCredentialSource {
    fn get_credentials(
        &self,
        custom_llm_provider: &str,
        region_name: Option<&str>,
    ) -> impl Future<Output = Option<SecretString>> + Send;
}

// The transcript GET the poll sends, with the timeout of the client that serves the relay
pub trait AssemblyTranscriptTransport {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub trait AssemblySleeper {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

// PassThroughEndpointLogging._handle_logging's arguments plus the spend LiteLLM writes first
#[derive(Debug)]
pub struct AssemblyTranscriptLog {
    pub response: PassThroughResponse,
    pub spend: MediaSpend,
    pub result: String,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: PassThroughLoggingKwargs,
}

pub trait AssemblyTranscriptLogSink {
    fn handle_logging(&self, call: AssemblyTranscriptLog) -> impl Future<Output = ()> + Send;
}

// The success handler's arguments for one relayed request, owned so the poll outlives the response
#[derive(Debug)]
pub struct AssemblyLogCall {
    pub request_method: String,
    pub response_body: Json,
    pub url_route: Option<String>,
    pub result: String,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: PassThroughLoggingKwargs,
}

pub struct AssemblyAiPorts<Credentials, Transport, Pricing, Sleeper, Sink> {
    pub credentials: Credentials,
    pub transport: Transport,
    pub pricing: Pricing,
    pub sleeper: Sleeper,
    pub sink: Sink,
}

pub struct AssemblyAiLogging<Credentials, Transport, Pricing, Sleeper, Sink> {
    ports: AssemblyAiPorts<Credentials, Transport, Pricing, Sleeper, Sink>,
    settings: AssemblyAiSettings,
    tasks: TaskTracker,
}

impl<Credentials, Transport, Pricing, Sleeper, Sink>
    AssemblyAiLogging<Credentials, Transport, Pricing, Sleeper, Sink>
where
    Credentials: AssemblyCredentialSource + Send + Sync + 'static,
    Transport: AssemblyTranscriptTransport + Send + Sync + 'static,
    Pricing: PriceLookup + Send + Sync + 'static,
    Sleeper: AssemblySleeper + Send + Sync + 'static,
    Sink: AssemblyTranscriptLogSink + Send + Sync + 'static,
{
    #[must_use]
    pub const fn new(
        ports: AssemblyAiPorts<Credentials, Transport, Pricing, Sleeper, Sink>,
        settings: AssemblyAiSettings,
        tasks: TaskTracker,
    ) -> Self {
        Self {
            ports,
            settings,
            tasks,
        }
    }

    // The executor.submit of the success handler: the relayed response never waits for the poll
    pub fn log(self: &Arc<Self>, call: AssemblyLogCall) {
        if call.request_method != "POST" {
            return;
        }
        let Ok(handle) = Handle::try_current() else {
            tracing::error!(
                "AssemblyAI transcript logging has no async runtime to poll in; the spend stays unlogged"
            );
            return;
        };
        let service = Arc::clone(self);
        self.tasks.spawn_on(
            async move {
                if let Err(error) = service.run(&call).await {
                    tracing::error!(%error, "AssemblyAI transcript logging failed");
                }
            },
            &handle,
        );
    }

    // _handle_assemblyai_passthrough_logging: read the submitted job, poll it, price it, then dispatch
    pub async fn run(&self, call: &AssemblyLogCall) -> Result<(), AssemblyAiError> {
        let (spend, response) = self.poll_spend(call).await?;
        let mut kwargs = call.kwargs.clone();
        kwargs
            .entries
            .insert("model".to_owned(), Json::Text(spend.model.clone()));
        kwargs.entries.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(ASSEMBLY_AI_PROVIDER.to_owned()),
        );
        self.ports
            .sink
            .handle_logging(AssemblyTranscriptLog {
                response: PassThroughResponse::Transcription(response),
                spend,
                result: call.result.clone(),
                start_time: call.start_time,
                end_time: call.end_time,
                cache_hit: call.cache_hit,
                kwargs,
            })
            .await;
        Ok(())
    }

    // The submitted job's transcript, polled to an answer and priced by speech model
    async fn poll_spend(
        &self,
        call: &AssemblyLogCall,
    ) -> Result<(MediaSpend, Json), AssemblyAiError> {
        let empty = IndexMap::new();
        let body = match &call.response_body {
            Json::Object(fields) => fields,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &empty,
        };
        let Some(transcript_id) = transcript::text_field(body, "id")? else {
            return Err(AssemblyAiError::MissingTranscriptId);
        };
        let speech_model = transcript::text_field(body, "speech_model")?.unwrap_or_default();
        let region = assembly_region_from_url(call.url_route.as_deref());
        let poller = poll::TranscriptPoller {
            settings: &self.settings,
            credentials: &self.ports.credentials,
            sleeper: &self.ports.sleeper,
            transport: &self.ports.transport,
        };
        let transcript = poller.poll(transcript_id, region).await?;
        let response_cost = match &transcript {
            Some(fields) => pricing::transcript_cost(fields, speech_model, &self.ports.pricing)?,
            None => None,
        };
        let spend = MediaSpend {
            model: speech_model.to_owned(),
            custom_llm_provider: ASSEMBLY_AI_PROVIDER,
            response_cost,
        };
        let response = transcript.map_or_else(|| Json::Object(IndexMap::new()), Json::Object);
        Ok((spend, response))
    }
}

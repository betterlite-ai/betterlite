use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::HeaderMap;
use indexmap::IndexMap;
use tokio::time::sleep;
use url::Url;

use super::transcript::{encode_transcript_id, request_headers, text_field};
use super::{
    ASSEMBLY_AI_PROVIDER, AssemblyAiError, AssemblyAiSettings, AssemblyCredentialSource,
    AssemblySleeper, AssemblyTranscriptTransport,
};

// LiteLLM's poll passes no timeout, so the provider client's own read limit bounds each GET
impl AssemblyTranscriptTransport for ProxiedClient {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        self.send_buffered(OutboundRequest {
            method: Method::GET,
            url,
            query: Vec::new(),
            headers,
            body: RequestBody::Empty,
            deadline: self.limits.read_timeout,
        })
        .await
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TokioSleeper;

impl AssemblySleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        sleep(duration).await;
    }
}

// The provider's answer: a null body reads as absent, an object as the transcript, the rest is a shape error
pub fn decode_assembly_transcript(
    body: &[u8],
) -> Result<Option<IndexMap<String, Json>>, AssemblyAiError> {
    let Ok(answer) = serde_json::from_slice::<Json>(body) else {
        return Ok(None);
    };
    match answer {
        Json::Null => Ok(None),
        Json::Object(fields) => Ok(Some(fields)),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(AssemblyAiError::TranscriptShape)
        }
    }
}

// What one transcript poll needs: the provider bases, the key, the client and the wait between asks
pub(super) struct TranscriptPoller<'service, Credentials, Sleeper, Transport> {
    pub(super) settings: &'service AssemblyAiSettings,
    pub(super) credentials: &'service Credentials,
    pub(super) sleeper: &'service Sleeper,
    pub(super) transport: &'service Transport,
}

impl<Credentials, Sleeper, Transport> TranscriptPoller<'_, Credentials, Sleeper, Transport>
where
    Credentials: AssemblyCredentialSource + Send + Sync,
    Sleeper: AssemblySleeper + Send + Sync,
    Transport: AssemblyTranscriptTransport + Send + Sync,
{
    // _poll_assembly_for_transcript_response: an absent answer ends the poll, a non-terminal one waits
    pub(super) async fn poll(
        &self,
        transcript_id: &str,
        region: Option<&str>,
    ) -> Result<Option<IndexMap<String, Json>>, AssemblyAiError> {
        for _attempt in 0..self.settings.max_polling_attempts {
            let Some(transcript) = self.poll_once(transcript_id, region).await? else {
                return Ok(None);
            };
            let finished = matches!(
                text_field(&transcript, "status")?,
                Some("completed" | "error")
            );
            if finished {
                return Ok(Some(transcript));
            }
            self.sleeper.sleep(self.settings.polling_interval).await;
        }
        Ok(None)
    }

    // _get_assembly_transcript: every failure short of a missing key reads as an absent transcript
    async fn poll_once(
        &self,
        transcript_id: &str,
        region: Option<&str>,
    ) -> Result<Option<IndexMap<String, Json>>, AssemblyAiError> {
        let base = self.settings.polling_base(region);
        let Some(api_key) = self
            .credentials
            .get_credentials(ASSEMBLY_AI_PROVIDER, region)
            .await
        else {
            return Err(AssemblyAiError::MissingApiKey);
        };
        let encoded_id = encode_transcript_id(transcript_id)?;
        let Ok(url) = Url::parse(&format!("{base}/v2/transcript/{encoded_id}")) else {
            return Ok(None);
        };
        let Some(headers) = request_headers(&api_key) else {
            return Ok(None);
        };
        let Ok(response) = self.transport.get(url, headers).await else {
            return Ok(None);
        };
        if response.status >= 400 {
            return Ok(None);
        }
        decode_assembly_transcript(&response.body)
    }
}

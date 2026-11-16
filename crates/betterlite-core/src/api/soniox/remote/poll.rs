use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::openai_base_config::Headers;
use http::Method;
use rust_decimal::Decimal;

use super::SonioxRemote;
use super::error::RemoteError;
use super::exchange::{endpoint_url, header_map, interval, interval_text, reply};
use super::transport::{RemoteTransport, Sleeper};
use crate::api::soniox::prepare::decode::SonioxReply;
use crate::api::soniox::prepare::raise::raise_for_response;
use crate::api::soniox::prepare::response::{TranscriptionMeta, transcription_meta};
use crate::api::soniox::records::error::SonioxException;

pub const POLL_ACTION: &str = "poll transcription";
pub const COMPLETED: &str = "completed";
pub const ERROR: &str = "error";
pub const UNKNOWN_DETAIL: &str = "unknown error";
pub const FAILED_STATUS: i64 = 500;
pub const TIMEOUT_STATUS: i64 = 504;

impl<T: RemoteTransport, S: Sleeper> SonioxRemote<'_, T, S> {
    pub async fn poll(&self, transcription_id: &str) -> Result<TranscriptionMeta, RemoteError> {
        let wait = interval(self.prepared.options.poll_interval)?;
        let attempts = self.prepared.options.max_attempts;
        for _ in 0..attempts {
            let decoded = self.poll_once(transcription_id).await?;
            let meta = transcription_meta(&decoded)?;
            match meta.status.as_deref() {
                Some(COMPLETED) => return Ok(meta),
                Some(ERROR) => {
                    return Err(RemoteError::Provider(failure(
                        transcription_id,
                        &meta,
                        &decoded,
                    )));
                }
                None | Some(_) => self.sleeper.sleep(wait).await,
            }
        }
        Err(RemoteError::Provider(timeout(
            transcription_id,
            attempts,
            self.prepared.options.poll_interval,
        )))
    }

    async fn poll_once(&self, transcription_id: &str) -> Result<SonioxReply, RemoteError> {
        let request = OutboundRequest {
            method: Method::GET,
            url: endpoint_url(&format!(
                "{}/v1/transcriptions/{transcription_id}",
                self.prepared.api_base
            ))?,
            query: Vec::new(),
            headers: header_map(&self.prepared.auth_headers)?,
            body: RequestBody::Empty,
            deadline: self.timeout,
        };
        let response = self.transport.call(request).await?;
        let decoded = reply(response, &self.limits)?;
        raise_for_response(&decoded, POLL_ACTION)?;
        Ok(decoded)
    }
}

fn failure(
    transcription_id: &str,
    meta: &TranscriptionMeta,
    decoded: &SonioxReply,
) -> SonioxException {
    let detail = meta
        .error_message
        .as_deref()
        .filter(|message| !message.is_empty())
        .or_else(|| meta.error_type.as_deref().filter(|kind| !kind.is_empty()))
        .unwrap_or(UNKNOWN_DETAIL);
    SonioxException {
        message: format!("Soniox transcription {transcription_id} failed: {detail}"),
        status_code: FAILED_STATUS,
        headers: Some(decoded.headers.clone()),
    }
}

fn timeout(transcription_id: &str, attempts: i64, wait: Decimal) -> SonioxException {
    SonioxException {
        message: format!(
            "Soniox transcription {transcription_id} did not complete after {attempts} polling attempts (interval={}s).",
            interval_text(wait)
        ),
        status_code: TIMEOUT_STATUS,
        headers: Some(Headers::new()),
    }
}

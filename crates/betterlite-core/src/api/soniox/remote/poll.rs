use betterlite_openai::openai_base_config::Headers;
use rust_decimal::Decimal;

use super::SonioxRemote;
use super::error::RemoteError;
use super::exchange::{interval, interval_text, reply, transcription_request};
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

pub async fn poll_until_completed<T: RemoteTransport, S: Sleeper>(
    remote: &SonioxRemote<'_, T, S>,
    transcription_id: &str,
) -> Result<TranscriptionMeta, RemoteError> {
    let wait = interval(remote.prepared.options.poll_interval)?;
    let attempts = remote.prepared.options.max_attempts;
    for _ in 0..attempts {
        let request = transcription_request(remote.prepared, transcription_id, remote.timeout)?;
        let response = remote.transport.call(request).await?;
        let decoded = reply(&response, &remote.limits)?;
        raise_for_response(&decoded, POLL_ACTION)?;
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
            None | Some(_) => remote.sleeper.sleep(wait).await,
        }
    }
    Err(RemoteError::Provider(timeout(
        transcription_id,
        attempts,
        remote.prepared.options.poll_interval,
    )))
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

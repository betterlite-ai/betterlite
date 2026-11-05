use std::io::Write as _;
use std::time::Duration;

use flate2::Compression;
use flate2::write::GzEncoder;

use super::destination::FocusMavvrikDestination;
use super::error::MavvrikDeliveryError;
use super::transport::{Method, Transport, TransportRequest};
use super::validation::validate_gcs_url;

// GCS requires intermediate chunks to be a multiple of 256 KiB
const CHUNK_SIZE: usize = 8 * 1024 * 1024;
const SESSION_INIT_TIMEOUT: Duration = Duration::from_secs(30);
const CHUNK_TIMEOUT: Duration = Duration::from_secs(120);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);
const GCS_ERROR_BODY_BYTES: usize = 400;
const SESSION_METADATA: &[u8] = br#"{"contentEncoding":"gzip","contentDisposition":"attachment"}"#;

pub(super) async fn upload_to_gcs<Http>(
    destination: &FocusMavvrikDestination<Http>,
    signed_url: &str,
    content: &[u8],
) -> Result<(), MavvrikDeliveryError<Http::Error>>
where
    Http: Transport,
{
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(content)
        .map_err(MavvrikDeliveryError::Compress)?;
    let gzip_bytes = encoder.finish().map_err(MavvrikDeliveryError::Compress)?;

    let session_uri = start_session(destination, signed_url).await?;
    tracing::debug!(
        bytes = gzip_bytes.len(),
        "Mavvrik FOCUS destination: GCS session started"
    );
    let uploaded = upload_chunks(destination, &session_uri, &gzip_bytes).await;
    if uploaded.is_err() {
        // Best effort: an open session otherwise lingers on GCS for about a week
        let cancel = TransportRequest {
            method: Method::Delete,
            url: session_uri,
            headers: Vec::new(),
            api_key: None,
            body: Vec::new(),
            timeout: CANCEL_TIMEOUT,
        };
        if let Err(error) = destination.send(cancel).await {
            tracing::debug!(
                ?error,
                "Mavvrik FOCUS destination: could not cancel the GCS session"
            );
        }
    }
    uploaded
}

async fn start_session<Http>(
    destination: &FocusMavvrikDestination<Http>,
    signed_url: &str,
) -> Result<String, MavvrikDeliveryError<Http::Error>>
where
    Http: Transport,
{
    let init = TransportRequest {
        method: Method::Post,
        url: signed_url.to_owned(),
        headers: vec![
            ("Content-Type", "application/gzip".to_owned()),
            ("x-goog-resumable", "start".to_owned()),
        ],
        api_key: None,
        body: SESSION_METADATA.to_vec(),
        timeout: SESSION_INIT_TIMEOUT,
    };
    let response = destination.send(init).await?;
    if !matches!(response.status, 200 | 201) {
        return Err(MavvrikDeliveryError::SessionInitFailed {
            status: response.status,
            body: response.text_prefix(GCS_ERROR_BODY_BYTES),
        });
    }
    let session_uri = response
        .header("Location")
        .filter(|location| !location.is_empty())
        .ok_or(MavvrikDeliveryError::SessionLocationMissing)?;
    validate_gcs_url(session_uri, "session URI")?;
    Ok(session_uri.to_owned())
}

async fn upload_chunks<Http>(
    destination: &FocusMavvrikDestination<Http>,
    session_uri: &str,
    gzip_bytes: &[u8],
) -> Result<(), MavvrikDeliveryError<Http::Error>>
where
    Http: Transport,
{
    let total = gzip_bytes.len();
    let mut offset = 0_usize;
    for chunk in gzip_bytes.chunks(CHUNK_SIZE) {
        let next_offset = offset.saturating_add(chunk.len());
        let last_byte = next_offset.saturating_sub(1);
        let is_final = next_offset >= total;
        let total_range = if is_final {
            total.to_string()
        } else {
            "*".to_owned()
        };
        let content_range = format!("bytes {offset}-{last_byte}/{total_range}");
        let response = destination
            .send(chunk_request(session_uri, chunk, content_range))
            .await?;
        let expected = if is_final { "{200, 201}" } else { "{308}" };
        if !chunk_accepted(response.status, is_final) {
            return Err(MavvrikDeliveryError::ChunkFailed {
                offset,
                expected,
                status: response.status,
                body: response.text_prefix(GCS_ERROR_BODY_BYTES),
            });
        }
        offset = next_offset;
    }
    Ok(())
}

fn chunk_request(session_uri: &str, chunk: &[u8], content_range: String) -> TransportRequest {
    TransportRequest {
        method: Method::Put,
        url: session_uri.to_owned(),
        headers: vec![
            ("Content-Type", "application/gzip".to_owned()),
            ("Content-Range", content_range),
        ],
        api_key: None,
        body: chunk.to_vec(),
        timeout: CHUNK_TIMEOUT,
    }
}

const fn chunk_accepted(status: u16, is_final: bool) -> bool {
    if is_final {
        matches!(status, 200 | 201)
    } else {
        status == 308
    }
}

use super::core::PointFiveCore;
use super::error::PointFiveError;
use crate::finops::pointfive::error::PointFiveUploadError;
use crate::finops::pointfive::payload::{chunk_lines, encode_lines, serialize_records};

// async_send_batch: a retryable failure removes only the already shipped chunks, then raises
pub(crate) async fn send_batch(core: &PointFiveCore) -> Result<(), PointFiveError> {
    let pending = core.queue.snapshot();
    if pending.is_empty() {
        return Ok(());
    }
    let client = core.upload_client()?;
    let lines = serialize_records(&pending)?;
    let chunks = chunk_lines(&lines, core.params.max_batch_bytes);
    for (index, chunk) in chunks.iter().enumerate() {
        let body = encode_lines(chunk).await?;
        match client.upload(&body).await {
            Ok(_object_key) => {}
            Err(failure) if failure.retryable => {
                let shipped: usize = chunks.iter().take(index).map(Vec::len).sum();
                core.queue.remove_prefix(shipped);
                return Err(PointFiveError::Upload(PointFiveUploadError {
                    detail: failure.detail,
                }));
            }
            Err(failure) => {
                tracing::error!(
                    records = chunk.len(),
                    detail = %failure.detail,
                    "pointfive: dropping records the api refused"
                );
            }
        }
    }
    Ok(())
}

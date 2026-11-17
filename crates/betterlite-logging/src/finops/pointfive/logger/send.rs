use super::core::PointFiveCore;
use super::error::PointFiveError;
use crate::finops::pointfive::error::PointFiveUploadError;
use crate::finops::pointfive::payload::{chunk_lines, encode_lines, serialize_records};

impl PointFiveCore {
    // async_send_batch: a retryable failure removes only the already shipped chunks, then raises
    pub async fn send_batch(&self) -> Result<(), PointFiveError> {
        let pending = self.queue.snapshot();
        if pending.is_empty() {
            return Ok(());
        }
        let client = self.upload_client()?;
        let lines = serialize_records(&pending)?;
        let chunks = chunk_lines(&lines, self.params.max_batch_bytes);
        for (index, chunk) in chunks.iter().enumerate() {
            let body = encode_lines(chunk).await?;
            match client.upload(&body).await {
                Ok(_object_key) => {}
                Err(failure) if failure.retryable => {
                    let shipped: usize = chunks.iter().take(index).map(Vec::len).sum();
                    self.queue.remove_prefix(shipped);
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
}

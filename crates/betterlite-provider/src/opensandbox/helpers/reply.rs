use futures_util::TryStreamExt as _;
use futures_util::stream::BoxStream;

use super::OpenSandboxError;

pub(super) const MAX_RESPONSE_BYTES: usize = 0x0100_0000;

pub const fn check_status(status: u16) -> Result<(), OpenSandboxError> {
    match status {
        200..=299 => Ok(()),
        _ => Err(OpenSandboxError::HttpStatus { status }),
    }
}

pub(super) async fn collect_reply(
    reply: BoxStream<'static, Result<Vec<u8>, OpenSandboxError>>,
) -> Result<Vec<u8>, OpenSandboxError> {
    reply
        .try_fold(Vec::new(), |mut collected, piece| async move {
            let total = collected.len().saturating_add(piece.len());
            if total > MAX_RESPONSE_BYTES {
                return Err(OpenSandboxError::ResponseTooLarge {
                    limit: MAX_RESPONSE_BYTES,
                });
            }
            collected.extend_from_slice(&piece);
            Ok(collected)
        })
        .await
}

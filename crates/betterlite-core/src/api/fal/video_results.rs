pub mod content;
pub mod error;
pub mod status;
pub mod text;
pub mod transport;
pub mod unsupported;

mod tests;

use crate::api::fal::video_prepare::VideoHeaders;

// the fal.ai answer the proxy received; the retrieval methods decode it once and never re-parse the body
pub struct UpstreamReply {
    pub status: u16,
    pub headers: VideoHeaders,
    pub body: Vec<u8>,
}

pub mod classifier;
pub mod messages;
pub mod placeholder;
pub mod redaction;
pub mod request;
pub mod response;
pub mod session;
pub mod support;
pub mod vector_store;
use crate::spend_sanitize::SpendSanitizePolicy;
// The caller resolves storage, truncation and global privacy from config, so this layer reads no server settings
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpendStoragePolicy {
    pub sanitize: SpendSanitizePolicy,
    pub turn_off_message_logging: bool,
}
mod tests;

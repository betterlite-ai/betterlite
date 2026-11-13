pub mod codex;
pub mod environment;
pub mod error;
pub mod messages;
pub mod pipeline;
pub mod prepare;
pub mod resolver;
pub mod sync;
use std::ffi::OsStr;
use std::path::Path;
pub(crate) fn binary_name(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(command)
}
mod tests;

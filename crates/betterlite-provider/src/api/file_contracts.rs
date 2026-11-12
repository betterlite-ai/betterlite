pub mod base;
pub mod config;
pub mod endpoints;
pub mod error;
mod tests;
pub mod upload_stream;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileRequest;
pub struct FileResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FilePrepared;

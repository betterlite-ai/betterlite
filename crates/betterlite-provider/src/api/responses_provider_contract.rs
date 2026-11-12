pub mod config;
pub mod error;
pub mod records;
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponsesRequest;
pub struct ResponsesResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponsesPrepared;

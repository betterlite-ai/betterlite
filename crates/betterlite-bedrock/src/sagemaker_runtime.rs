pub mod chat;
pub mod completion;
mod convert;
pub mod embedding;
pub mod endpoint;
pub mod error;
pub mod runtime;

mod tests;

pub const CONTENT_TYPE: &str = "Content-Type";
pub const JSON_CONTENT_TYPE: &str = "application/json";

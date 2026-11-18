#![expect(
    clippy::disallowed_types,
    reason = "Chat options and the Responses request carry provider JSON such as tools, response_format and extra_body, which has no closed schema"
)]

pub mod declared;
pub mod error;
pub mod extra_body;
pub mod options;
pub mod records;
pub mod request;
pub mod text_format;
pub mod tools;

mod tests;

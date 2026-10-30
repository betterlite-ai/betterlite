pub mod decode;
pub mod errors;
pub mod fields;
pub mod frame;
mod tests;
pub mod wire;

use wire::ErrorRecord;

pub struct AnthropicExceptionMapping;

#[derive(Debug, Clone)]
pub struct AnthropicErrorSseFrame {
    pub status_code: u16,
    pub error_response: ErrorRecord,
    pub body: String,
}

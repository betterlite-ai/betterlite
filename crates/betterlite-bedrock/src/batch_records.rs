// Bedrock batch records: an OpenAI batch JSONL upload becomes the InvokeModel records Bedrock runs.
pub mod chat_body;
pub mod error;
pub mod families;
pub mod jsonl;
pub mod params;
pub mod record;
pub mod resolve;
pub mod upload;

mod tests;

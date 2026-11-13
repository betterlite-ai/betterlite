pub mod capabilities;
pub mod credentials;
pub mod error;
pub mod messages;
pub mod options;
pub mod reply;
pub mod stream;

mod tests;

pub const PROVIDER: &str = "groq";
pub const DEFAULT_API_BASE: &str = "https://api.groq.com/openai/v1";

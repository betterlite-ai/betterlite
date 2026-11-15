pub mod error;
pub mod headers;
pub mod params;
pub mod response;
pub mod url;

mod tests;

use betterlite_tokenizer::count::error::CountError;
use uuid::Uuid;

pub trait TextTokenCounter: Send + Sync {
    fn count(&self, model: &str, text: &str) -> Result<u64, CountError>;
}

pub trait RequestIdSource: Send + Sync {
    fn new_id(&self) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Uuid4;

impl RequestIdSource for Uuid4 {
    fn new_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}

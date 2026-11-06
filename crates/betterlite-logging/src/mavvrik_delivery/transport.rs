use std::error::Error;
use std::future::Future;
use std::time::Duration;

use secrecy::SecretString;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Delete,
    Get,
    Patch,
    Post,
    Put,
}

#[derive(Debug)]
pub struct TransportRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    // Sent as x-api-key, so the key stays a SecretString until the wire
    pub api_key: Option<SecretString>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct TransportResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl TransportResponse {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    #[must_use]
    pub fn text_prefix(&self, limit: usize) -> String {
        // An error page that is not UTF-8 is cut where the valid text ends
        let text = match str::from_utf8(&self.body) {
            Ok(text) => text,
            Err(invalid) => {
                str::from_utf8(self.body.get(..invalid.valid_up_to()).unwrap_or_default())
                    .unwrap_or_default()
            }
        };
        text.get(..text.floor_char_boundary(limit))
            .unwrap_or_default()
            .to_owned()
    }
}

pub trait Transport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    // An implementation enforces request.timeout and reports an elapsed one as its own error
    fn send(
        &self,
        request: TransportRequest,
    ) -> impl Future<Output = Result<TransportResponse, Self::Error>> + Send;
}

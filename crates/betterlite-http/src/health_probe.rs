use std::io::{self, Read as _};
use std::string::FromUtf8Error;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect::Policy;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEALTH_BODY: u64 = 0x10_0000;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error("reading the health response failed: {0}")]
    Read(#[from] io::Error),
    #[error("the health response is longer than {limit} bytes")]
    BodyTooLarge { limit: u64 },
    #[error("the health response is not valid UTF-8: {0}")]
    NotUtf8(#[from] FromUtf8Error),
}

pub trait HealthTransport {
    fn get(&self, url: &str) -> Result<String, TransportError>;
}

pub struct ReqwestHealth {
    client: Client,
}

impl ReqwestHealth {
    pub fn new() -> Result<Self, TransportError> {
        let client = Client::builder()
            .timeout(HEALTH_TIMEOUT)
            .redirect(Policy::none())
            .build()?;
        Ok(Self { client })
    }
}

impl HealthTransport for ReqwestHealth {
    fn get(&self, url: &str) -> Result<String, TransportError> {
        let response = self.client.get(url).send()?;
        let mut body = Vec::new();
        let mut capped = response.take(MAX_HEALTH_BODY + 1);
        capped.read_to_end(&mut body)?;
        if capped.limit() == 0 {
            return Err(TransportError::BodyTooLarge {
                limit: MAX_HEALTH_BODY,
            });
        }
        Ok(String::from_utf8(body)?)
    }
}

mod tests;

use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect::Policy;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
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
        Ok(response.text()?)
    }
}

mod tests;

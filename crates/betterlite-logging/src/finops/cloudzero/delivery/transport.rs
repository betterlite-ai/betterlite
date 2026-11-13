use std::error::Error as StdError;
use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};
use secrecy::SecretString;

use super::payload::BillingDrop;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug)]
pub struct BillingDropRequest<'request> {
    pub url: SecretString,
    pub authorization: SecretString,
    pub timeout: Duration,
    pub body: &'request BillingDrop<'request>,
}

// An implementation POSTs the JSON body and fails on a network error or a non-2xx status
pub trait BillingTransport {
    type Error: StdError + Send + Sync + 'static;

    fn post<'request>(
        &'request self,
        request: &'request BillingDropRequest<'request>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'request;
}

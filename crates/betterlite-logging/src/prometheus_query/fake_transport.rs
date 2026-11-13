#![cfg(test)]

use super::PrometheusQuery;
use super::PrometheusSettings;
use super::clock::Clock;
use super::transport::{QueryRequest, Transport};
use chrono::{DateTime, TimeZone as _, Utc};
use std::collections::VecDeque;
use std::future::ready;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
#[error("the fake Prometheus server refused the request")]
pub struct FakeFailure;

#[derive(Debug)]
pub struct FakeTransport {
    pub responses: Mutex<VecDeque<Result<Vec<u8>, FakeFailure>>>,
    pub requests: Mutex<Vec<QueryRequest>>,
}

impl Transport for FakeTransport {
    type Error = FakeFailure;

    fn get(
        &self,
        request: QueryRequest,
    ) -> impl Future<Output = Result<Vec<u8>, FakeFailure>> + Send {
        self.requests.lock().unwrap().push(request);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(FakeFailure));
        ready(response)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub fn prometheus_query(
    url: Option<&str>,
    selected_instance: Option<&str>,
    responses: Vec<Result<Vec<u8>, FakeFailure>>,
) -> PrometheusQuery<FakeTransport, FixedClock> {
    PrometheusQuery {
        settings: PrometheusSettings {
            url: url.map(str::to_owned),
            selected_instance: selected_instance.map(str::to_owned),
        },
        transport: FakeTransport {
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
        },
        clock: FixedClock(Utc.timestamp_opt(1_724_000_000, 250_000_000).unwrap()),
    }
}

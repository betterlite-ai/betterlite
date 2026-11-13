#![cfg(test)]

use std::future::{Future, ready};
use std::io;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use chrono::{DateTime, TimeZone as _, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::subscriber::with_default;

use crate::finops::cloudzero::delivery::payload::ApiRecord;
use crate::finops::cloudzero::delivery::transport::{BillingDropRequest, BillingTransport, Clock};
use crate::finops::cloudzero::delivery::{CloudZeroStreamer, Credentials};
use crate::finops::cloudzero::records::{CbfRecord, CbfValue};

pub(super) const API_KEY: &str = "cz-api-key-1234";
pub(super) const CONNECTION_ID: &str = "conn-5678";

#[derive(Debug, thiserror::Error)]
#[error("the fake transport refused the post")]
pub(super) struct Refused;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PostedBody {
    pub month: String,
    pub operation: String,
    pub data: Vec<ApiRecord>,
}

#[derive(Clone, Debug)]
pub(super) struct Posted {
    pub url: String,
    pub authorization: String,
    pub timeout: Duration,
    pub body: PostedBody,
    pub request_debug: String,
}

#[derive(Clone, Debug, Default)]
pub(super) struct FakeTransport {
    pub posted: Arc<Mutex<Vec<Posted>>>,
    pub fail_on_call: Option<usize>,
}

impl BillingTransport for FakeTransport {
    type Error = Refused;

    fn post<'request>(
        &'request self,
        request: &'request BillingDropRequest<'request>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'request {
        let call = self.posted.lock().unwrap().len();
        self.posted.lock().unwrap().push(Posted {
            url: request.url.expose_secret().to_owned(),
            authorization: request.authorization.expose_secret().to_owned(),
            timeout: request.timeout,
            body: PostedBody {
                month: request.body.month.clone(),
                operation: request.body.operation.to_owned(),
                data: request.body.data.clone(),
            },
            request_debug: format!("{request:?}"),
        });
        ready(if self.fail_on_call == Some(call) {
            Err(Refused)
        } else {
            Ok(())
        })
    }
}

#[derive(Debug)]
pub(super) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2030, 1, 2, 3, 4, 5).unwrap()
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture_logs<Output, Run>(run: Run) -> (Output, String)
where
    Run: FnOnce() -> Output,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let output = with_default(subscriber, run);
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    (output, logs)
}

pub(super) fn streamer(
    user_timezone: Option<&str>,
    transport: FakeTransport,
) -> CloudZeroStreamer<FakeTransport, FixedClock> {
    CloudZeroStreamer::new(
        Credentials {
            api_key: SecretString::from(API_KEY),
            connection_id: SecretString::from(CONNECTION_ID),
        },
        user_timezone,
        &TimeZoneDatabase::from_env(),
        FixedClock,
        transport,
    )
}

pub(super) fn row(start: CbfValue, cost: Decimal) -> CbfRecord {
    CbfRecord::from([
        ("time/usage_start".to_owned(), start),
        ("cost/cost".to_owned(), CbfValue::Money(cost)),
        ("usage/amount".to_owned(), CbfValue::Integer(42)),
        (
            "resource/id".to_owned(),
            CbfValue::Text("czrn:x".to_owned()),
        ),
        ("resource/tag:team".to_owned(), CbfValue::None),
    ])
}

pub(super) fn text_row(start: &str) -> CbfRecord {
    row(CbfValue::Text(start.to_owned()), Decimal::new(15, 1))
}

pub(super) fn complete<Task>(task: Task) -> Task::Output
where
    Task: Future,
{
    let Poll::Ready(output) = pin!(task).poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("the fake transport completes immediately");
    };
    output
}

pub(super) fn posts_of(
    user_timezone: Option<&str>,
    data: &[CbfRecord],
    operation: Option<&str>,
) -> Vec<Posted> {
    let transport = FakeTransport::default();
    let posted = Arc::clone(&transport.posted);
    complete(streamer(user_timezone, transport).send_batched(data, operation)).unwrap();
    posted.lock().unwrap().clone()
}

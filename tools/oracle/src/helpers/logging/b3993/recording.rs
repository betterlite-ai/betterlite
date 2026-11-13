use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use betterlite_logging::s3_transport::error::S3TransportError;
use betterlite_logging::s3_transport::signing::Clock;
use betterlite_logging::s3_transport::transport::{
    S3HttpClient, S3Request, S3Response, ThrottleSink,
};
use serde::Serialize;

use super::input::Reply;

#[derive(Debug, Default)]
pub(crate) struct ScriptedS3 {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<S3Request>>,
}

impl ScriptedS3 {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn sent(&self) -> Vec<S3Request> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl S3HttpClient for ScriptedS3 {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        Box::pin(async move {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request);
            let reply = self
                .replies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .unwrap_or(Reply {
                    status: 200,
                    body: String::new(),
                });
            Ok(S3Response {
                status: reply.status,
                body: reply.body.into_bytes(),
            })
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct RecordingSink {
    samples: Mutex<Vec<bool>>,
}

impl RecordingSink {
    pub(crate) fn samples(&self) -> Vec<bool> {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ThrottleSink for RecordingSink {
    fn record(&self, throttled: bool) {
        self.samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(throttled);
    }
}

#[derive(Debug)]
pub(crate) struct FixedClock(pub(crate) SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Serialize)]
pub(crate) struct Recorded {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) body: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
}

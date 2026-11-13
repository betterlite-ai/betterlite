use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};

use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::vantage::destination::VantageDestination;
use crate::finops::vantage::error::DeliveryError;
use crate::finops::vantage::sender::{CsvSender, MultipartPost};
use crate::finops::vantage::settings::VantageSettings;
use crate::focus_records::destinations::{FocusDestination as _, FocusTimeWindow};

pub(super) const HEADER: &str = "ServiceName";
pub(super) const FILENAME: &str = "usage.csv";
pub(super) const SIZE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Posted {
    pub(super) url: String,
    pub(super) authorization: String,
    pub(super) field: String,
    pub(super) filename: String,
    pub(super) media_type: String,
    pub(super) data: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
#[error("the network is down for {0}")]
pub(super) struct NetworkDown(pub(super) String);

#[derive(Clone, Default)]
pub(super) struct FakeSender {
    pub(super) posted: Arc<Mutex<Vec<Posted>>>,
    pub(super) failing: Vec<String>,
    pub(super) status: Option<u16>,
}

impl FakeSender {
    pub(super) fn failing_on(failing: Vec<String>) -> Self {
        Self {
            posted: Arc::default(),
            failing,
            status: None,
        }
    }

    pub(super) fn answering(status: u16) -> Self {
        Self {
            posted: Arc::default(),
            failing: Vec::new(),
            status: Some(status),
        }
    }
}

impl CsvSender for FakeSender {
    type Error = NetworkDown;

    fn post_multipart(
        &self,
        post: MultipartPost,
    ) -> impl Future<Output = Result<u16, NetworkDown>> + Send {
        let filename = post.file.file_name.unwrap_or_default();
        self.posted.lock().unwrap().push(Posted {
            url: post.url.expose_secret().to_owned(),
            authorization: post.authorization.expose_secret().to_owned(),
            field: post.file.name,
            filename: filename.clone(),
            media_type: post.file.content_type.unwrap_or_default(),
            data: post.file.data,
        });
        let outcome = if self.failing.contains(&filename) {
            Err(NetworkDown(filename))
        } else {
            Ok(self.status.unwrap_or(200))
        };
        ready(outcome)
    }
}

pub(super) fn destination(sender: FakeSender) -> VantageDestination<FakeSender> {
    let settings = VantageSettings {
        api_key: Some(SecretString::from("key-1")),
        integration_token: Some(SecretString::from("token-1")),
        base_url: None,
    };
    VantageDestination::new(sender, settings).unwrap()
}

pub(super) fn rows(count: usize, width: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("{index:06}{}", "x".repeat(width.saturating_sub(6))))
        .collect()
}

pub(super) fn csv(header: &str, rows: &[String]) -> Vec<u8> {
    format!("{header}\n{}\n", rows.join("\n")).into_bytes()
}

pub(super) fn expected(ranges: &[(&str, usize, usize)], rows: &[String]) -> Vec<(String, Vec<u8>)> {
    ranges
        .iter()
        .map(|(suffix, from, to)| {
            (
                format!("{FILENAME}{suffix}"),
                csv(HEADER, &rows[*from..*to]),
            )
        })
        .collect()
}

pub(super) fn uploads(posted: &[Posted]) -> Vec<(String, Vec<u8>)> {
    posted
        .iter()
        .map(|post| (post.filename.clone(), post.data.clone()))
        .collect()
}

pub(super) fn window() -> FocusTimeWindow {
    FocusTimeWindow {
        start_time: DateTime::from_timestamp(0, 0).unwrap(),
        end_time: DateTime::from_timestamp(3_600, 0).unwrap(),
        frequency: "daily".to_owned(),
    }
}

pub(super) async fn deliver(
    sender: FakeSender,
    content: &[u8],
) -> (Result<(), DeliveryError<NetworkDown>>, Vec<Posted>) {
    let shared = Arc::clone(&sender.posted);
    let result = destination(sender)
        .deliver(content, &window(), FILENAME)
        .await;
    let posted = shared.lock().unwrap().clone();
    (result, posted)
}

#[derive(Clone, Default)]
pub(super) struct Captured(pub(super) Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut captured = self
            .0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?;
        captured.extend_from_slice(buf);
        drop(captured);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(super) fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
        let writer = self.clone();
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .finish()
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

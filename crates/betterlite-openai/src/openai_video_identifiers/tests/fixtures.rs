use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

pub(crate) const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
pub(crate) const JPEG: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00";
pub(crate) const GIF: &[u8] = b"GIF89a\x01\x00";
pub(crate) const WEBP: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";
pub(crate) const HEIC: &[u8] = b"\x00\x00\x00\x18ftypheic";

#[derive(Debug, Clone, Copy)]
pub(crate) enum Failure {
    Tell,
    Seek,
    Read,
    Restore,
}

pub(crate) struct ProbeReader {
    data: Vec<u8>,
    position: usize,
    failure: Option<Failure>,
    seeks: usize,
}

impl ProbeReader {
    pub(crate) fn new(data: &[u8], position: usize, failure: Option<Failure>) -> Self {
        Self {
            data: data.to_vec(),
            position,
            failure,
            seeks: 0,
        }
    }

    pub(crate) fn position(&self) -> usize {
        self.position
    }

    fn advance(&mut self, count: usize) -> usize {
        let remaining = self.data.len().saturating_sub(self.position);
        let taken = remaining.min(count);
        self.position = self.position.saturating_add(taken);
        taken
    }
}

impl Read for ProbeReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if matches!(self.failure, Some(Failure::Read)) {
            return Err(io::Error::other("read failed"));
        }
        let taken = self.advance(buffer.len());
        let start = self.position.saturating_sub(taken);
        let source = self.data.get(start..self.position).unwrap_or_default();
        buffer
            .get_mut(..taken)
            .unwrap_or_default()
            .copy_from_slice(source);
        Ok(taken)
    }
}

impl Seek for ProbeReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        self.seeks = self.seeks.saturating_add(1);
        if matches!(self.failure, Some(Failure::Seek)) && self.seeks == 1 {
            return Err(io::Error::other("seek failed"));
        }
        if matches!(self.failure, Some(Failure::Restore)) && self.seeks > 1 {
            return Err(io::Error::other("restore failed"));
        }
        if let SeekFrom::Start(offset) = target {
            self.position =
                usize::try_from(offset).map_err(|error| io::Error::other(error.to_string()))?;
        }
        Ok(u64::try_from(self.position).unwrap_or_default())
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        if matches!(self.failure, Some(Failure::Tell)) {
            return Err(io::Error::other("tell failed"));
        }
        Ok(u64::try_from(self.position).unwrap_or_default())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Capture(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

impl Capture {
    pub(crate) fn events(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn captured<T, F: FnOnce() -> T>(&self, work: F) -> T {
        let subscriber = tracing_subscriber::registry().with(self.clone());
        with_default(subscriber, work)
    }
}

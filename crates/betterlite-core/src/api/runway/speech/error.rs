use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::runway::tasks::error::RunwayTaskError;

#[derive(Debug, Error)]
pub enum RunwaySpeechError {
    #[error(transparent)]
    Task(#[from] RunwayTaskError),
    #[error("the RunwayML speech submission answer is not a task")]
    Submission {
        status: u16,
        headers: IndexMap<String, String>,
        #[source]
        source: Box<RunwayTaskError>,
    },
    #[error("RunwayML TTS response missing task ID")]
    MissingTaskId,
    #[error("RunwayML TTS response missing audio URL in output")]
    MissingAudioUrl,
    #[error("RunwayML TTS audio URL is not a string")]
    AudioUrlNotString,
    #[error("the RunwayML TTS audio URL is not a valid URL")]
    AudioUrl(#[source] url::ParseError),
    #[error("a RunwayML TTS header value holds a character a header cannot carry")]
    Header(#[source] InvalidHeaderValue),
    #[error("the RunwayML TTS audio download failed")]
    Download(#[source] TransportError),
    #[error("the RunwayML TTS audio download answered HTTP {status}")]
    DownloadStatus { status: u16 },
    #[error("the RunwayML speech option `{option}` must be a string")]
    InvalidOption { option: &'static str },
    #[error("{message}")]
    Handler {
        class: String,
        status: Option<u16>,
        message: String,
    },
}

impl RunwaySpeechError {
    // the parse failure builds the source's error class, so it is the one failure that keeps the submission's status
    #[must_use]
    pub const fn submission_status(&self) -> Option<u16> {
        match self {
            Self::Submission { status, .. } => Some(*status),
            Self::Task(_)
            | Self::MissingTaskId
            | Self::MissingAudioUrl
            | Self::AudioUrlNotString
            | Self::AudioUrl(_)
            | Self::Header(_)
            | Self::Download(_)
            | Self::DownloadStatus { .. }
            | Self::InvalidOption { .. }
            | Self::Handler { .. } => None,
        }
    }
}

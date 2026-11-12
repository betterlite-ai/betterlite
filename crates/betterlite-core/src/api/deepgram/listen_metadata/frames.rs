use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde::Deserialize;

// numbers stay apart from bools: Python's isinstance check rejects a bool where an int is declared
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
pub enum ListenNumber {
    Integer(i64),
    Float(f64),
    Bool(bool),
}

impl ListenNumber {
    fn count(self) -> Option<u64> {
        match self {
            Self::Integer(value) => u64::try_from(value).ok().filter(|count| *count >= 1),
            Self::Float(_) | Self::Bool(_) => None,
        }
    }

    fn seconds(self) -> Option<Decimal> {
        match self {
            Self::Integer(value) => {
                Some(Decimal::from(value)).filter(|seconds| *seconds >= Decimal::ZERO)
            }
            Self::Float(value) if value.is_finite() && value >= 0.0 => Decimal::from_f64(value),
            Self::Float(_) | Self::Bool(_) => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ListenFrame {
    Metadata(MetadataFrame),
    Results(ResultsFrame),
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct MetadataFrame {
    pub channels: Option<ListenNumber>,
    pub duration: Option<ListenNumber>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ResultsFrame {
    pub channel_index: Option<Vec<ListenNumber>>,
    pub start: Option<ListenNumber>,
    pub duration: Option<ListenNumber>,
    pub is_final: Option<bool>,
    pub channel: Option<ResultsChannel>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ResultsChannel {
    pub alternatives: Option<Vec<Alternative>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Alternative {
    pub transcript: Option<String>,
}

#[must_use]
pub fn channel_count(frames: &[ListenFrame], declared: Option<u64>) -> u64 {
    let mut metadata_channels = None;
    for frame in frames {
        if let ListenFrame::Metadata(metadata) = frame
            && let Some(count) = metadata.channels.and_then(ListenNumber::count)
        {
            metadata_channels = Some(count);
        }
    }
    if let Some(count) = metadata_channels {
        return count;
    }
    let mut results_channels = None;
    for frame in frames {
        if let ListenFrame::Results(results) = frame
            && let Some(count) = results_channel_count(results)
        {
            results_channels =
                Some(results_channels.map_or(count, |current: u64| current.max(count)));
        }
    }
    results_channels.or(declared).unwrap_or(1)
}

#[must_use]
pub fn audio_seconds(frames: &[ListenFrame]) -> Decimal {
    let mut metadata_duration = None;
    for frame in frames {
        if let ListenFrame::Metadata(metadata) = frame
            && let Some(seconds) = metadata.duration.and_then(ListenNumber::seconds)
            && seconds > Decimal::ZERO
        {
            metadata_duration = Some(seconds);
        }
    }
    if let Some(seconds) = metadata_duration {
        return seconds;
    }
    let mut longest = None;
    for frame in frames {
        if let ListenFrame::Results(results) = frame
            && let Some(end) = results_end(results)
        {
            longest = Some(longest.map_or(end, |current: Decimal| current.max(end)));
        }
    }
    longest.unwrap_or(Decimal::ZERO)
}

#[must_use]
pub fn transcript(frames: &[ListenFrame]) -> String {
    let mut parts = Vec::new();
    for frame in frames {
        if let ListenFrame::Results(results) = frame
            && results.is_final == Some(true)
            && let Some(text) = final_transcript(results)
        {
            parts.push(text);
        }
    }
    parts.join(" ")
}

fn results_channel_count(results: &ResultsFrame) -> Option<u64> {
    let channel_index = results.channel_index.as_ref()?;
    if channel_index.len() != 2 {
        return None;
    }
    channel_index.get(1)?.count()
}

fn results_end(results: &ResultsFrame) -> Option<Decimal> {
    let start = results.start.and_then(ListenNumber::seconds)?;
    let duration = results.duration.and_then(ListenNumber::seconds)?;
    start.checked_add(duration)
}

fn final_transcript(results: &ResultsFrame) -> Option<&str> {
    let alternative = results.channel.as_ref()?.alternatives.as_ref()?.first()?;
    let text = alternative.transcript.as_deref()?;
    (!text.is_empty()).then_some(text)
}

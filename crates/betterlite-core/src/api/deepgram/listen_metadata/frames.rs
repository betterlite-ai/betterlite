use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

// a field of the wrong type is no candidate and leaves the frame's other fields usable
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Lenient<T> {
    Value(T),
    Other(IgnoredAny),
}

impl<T> Lenient<T> {
    const fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Other(_) => None,
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

#[derive(Clone, Debug, Deserialize)]
pub struct MetadataFrame {
    pub channels: Option<Lenient<Number>>,
    pub duration: Option<Lenient<Number>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ResultsFrame {
    pub channel_index: Option<Lenient<Vec<Lenient<Number>>>>,
    pub start: Option<Lenient<Number>>,
    pub duration: Option<Lenient<Number>>,
    pub is_final: Option<Lenient<bool>>,
    pub channel: Option<Lenient<ResultsChannel>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ResultsChannel {
    pub alternatives: Option<Lenient<Vec<Lenient<Alternative>>>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Alternative {
    pub transcript: Option<Lenient<String>>,
}

// a bool or a string is no candidate; a negative or non-finite number is none either
fn candidate_seconds(field: Option<&Lenient<Number>>) -> Option<Decimal> {
    let number = field?.value()?;
    if let Some(whole) = number.as_u64() {
        return Some(Decimal::from(whole));
    }
    number
        .as_f64()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0_f64)
        .and_then(Decimal::from_f64)
}

#[must_use]
pub fn channel_count(frames: &[ListenFrame], declared: Option<u64>) -> u64 {
    let mut metadata_channels = None;
    for frame in frames {
        if let ListenFrame::Metadata(metadata) = frame
            && let Some(count) = metadata
                .channels
                .as_ref()
                .and_then(Lenient::value)
                .and_then(Number::as_u64)
                .filter(|count| *count >= 1)
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
            && let Some(seconds) = candidate_seconds(metadata.duration.as_ref())
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
            && matches!(results.is_final, Some(Lenient::Value(true)))
            && let Some(text) = final_transcript(results)
        {
            parts.push(text);
        }
    }
    parts.join(" ")
}

fn results_channel_count(results: &ResultsFrame) -> Option<u64> {
    let channel_index = results.channel_index.as_ref()?.value()?;
    if channel_index.len() != 2 {
        return None;
    }
    let number = channel_index.get(1)?.value()?;
    number.as_u64().filter(|count| *count >= 1)
}

fn results_end(results: &ResultsFrame) -> Option<Decimal> {
    let start = candidate_seconds(results.start.as_ref())?;
    let duration = candidate_seconds(results.duration.as_ref())?;
    start.checked_add(duration)
}

fn final_transcript(results: &ResultsFrame) -> Option<&str> {
    let channel = results.channel.as_ref()?.value()?;
    let alternatives = channel.alternatives.as_ref()?.value()?;
    let alternative = alternatives.first()?.value()?;
    let text = alternative.transcript.as_ref()?.value()?.as_str();
    (!text.is_empty()).then_some(text)
}

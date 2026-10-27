use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};

mod tests;

const METRICS_FIELD: &str = "amazon-bedrock-invocationMetrics";
const METRIC_FIELDS: [(&str, &str); 4] = [
    ("inputTokenCount", "input_tokens"),
    ("outputTokenCount", "output_tokens"),
    ("cacheReadInputTokenCount", "cache_read_input_tokens"),
    ("cacheWriteInputTokenCount", "cache_creation_input_tokens"),
];
const STOP_CACHE_FIELDS: [&str; 2] = ["cache_creation_input_tokens", "cache_read_input_tokens"];
const START_CACHE_FIELDS: [&str; 3] = [
    "cache_creation_input_tokens",
    "cache_read_input_tokens",
    "cache_creation",
];

// a provider record keeps its key order and every value's source text, so untouched fields pass through as sent
pub type Fields = IndexMap<String, Box<RawValue>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged, try_from = "Box<RawValue>")]
pub enum StreamItem {
    Event(Fields),
    Chunk(Box<RawValue>),
}

impl TryFrom<Box<RawValue>> for StreamItem {
    type Error = serde_json::Error;

    fn try_from(raw: Box<RawValue>) -> Result<Self, serde_json::Error> {
        Ok(object(&raw)?.map_or(Self::Chunk(raw), Self::Event))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MessageUsageError {
    #[error("a Bedrock message event field cannot be read or written: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Default)]
pub struct MessageUsageState {
    pending: Option<Fields>,
    start_usage: Option<Fields>,
}

pub fn promote_message_usage(items: Vec<StreamItem>) -> Result<Vec<StreamItem>, MessageUsageError> {
    let mut state = MessageUsageState::default();
    let mut output = Vec::with_capacity(items.len());
    for item in items {
        let decoded = match item {
            StreamItem::Event(fields) => StreamItem::Event(promote_invocation_metrics(fields)?),
            StreamItem::Chunk(raw) => StreamItem::Chunk(raw),
        };
        state.push(decoded, &mut output)?;
    }
    state.finish(&mut output)?;
    Ok(output)
}

pub fn promote_invocation_metrics(mut event: Fields) -> Result<Fields, MessageUsageError> {
    let Some(raw_metrics) = event.shift_remove(METRICS_FIELD) else {
        return Ok(event);
    };
    if !truthy(&raw_metrics) {
        return Ok(event);
    }
    let metrics = object(&raw_metrics)?.unwrap_or_default();
    let mut usage: Fields = METRIC_FIELDS
        .iter()
        .filter_map(|(metric, field)| {
            metrics
                .get(*metric)
                .map(|value| ((*field).to_owned(), value.clone()))
        })
        .collect();
    usage.extend(field_object(&event, "usage")?.unwrap_or_default());
    event.insert("usage".to_owned(), to_raw_value(&usage)?);
    Ok(event)
}

impl MessageUsageState {
    pub fn push(
        &mut self,
        item: StreamItem,
        output: &mut Vec<StreamItem>,
    ) -> Result<(), MessageUsageError> {
        let StreamItem::Event(event) = item else {
            output.extend(self.pending.take().map(StreamItem::Event));
            output.push(item);
            return Ok(());
        };
        match event_type(&event)?.as_deref() {
            Some("message_start") => {
                let message = field_object(&event, "message")?.unwrap_or_default();
                if let Some(usage) = field_object(&message, "usage")? {
                    self.start_usage = Some(usage);
                }
                output.extend(self.pending.take().map(StreamItem::Event));
            }
            Some("message_delta") => {
                self.pending = Some(event);
                return Ok(());
            }
            Some("message_stop") if let Some(delta) = self.pending.take() => {
                let stop_usage = field_object(&event, "usage")?.unwrap_or_default();
                output.push(StreamItem::Event(self.promote_stop(delta, &stop_usage)?));
            }
            _ => output.extend(self.pending.take().map(StreamItem::Event)),
        }
        output.push(StreamItem::Event(event));
        Ok(())
    }

    pub fn finish(mut self, output: &mut Vec<StreamItem>) -> Result<(), MessageUsageError> {
        if let Some(delta) = self.pending.take() {
            output.push(StreamItem::Event(self.promote_stop(delta, &Fields::new())?));
        }
        Ok(())
    }

    fn promote_stop(
        &self,
        mut delta: Fields,
        stop_usage: &Fields,
    ) -> Result<Fields, MessageUsageError> {
        let mut usage = field_object(&delta, "usage")?.unwrap_or_default();
        for field in STOP_CACHE_FIELDS {
            if let Some(value) = stop_usage.get(field) {
                usage.insert(field.to_owned(), value.clone());
            }
        }
        if let Some(input) = stop_usage.get("input_tokens")
            && input.get() != "null"
        {
            let text = input.get();
            // Python's isinstance(value, int) holds for booleans and for JSON numbers without a fraction or exponent
            let python_int = matches!(text, "true" | "false")
                || (matches!(text.as_bytes().first(), Some(b'-' | b'0'..=b'9'))
                    && !text.contains(['.', 'e', 'E']));
            let tokens = if python_int {
                input.clone()
            } else {
                to_raw_value(&0_u8)?
            };
            usage.insert("input_tokens".to_owned(), tokens);
        }
        for field in START_CACHE_FIELDS {
            if let Some(value) = self.start_usage.as_ref().and_then(|start| start.get(field))
                && !usage.contains_key(field)
                && value.get() != "null"
            {
                usage.insert(field.to_owned(), value.clone());
            }
        }
        if !usage.is_empty() {
            delta.insert("usage".to_owned(), to_raw_value(&usage)?);
        }
        Ok(delta)
    }
}

fn object(raw: &RawValue) -> Result<Option<Fields>, serde_json::Error> {
    if raw.get().starts_with('{') {
        serde_json::from_str(raw.get()).map(Some)
    } else {
        Ok(None)
    }
}

// LiteLLM reads `value or {}` and then dict methods, so anything but an object counts as absent here
fn field_object(record: &Fields, key: &str) -> Result<Option<Fields>, serde_json::Error> {
    let Some(value) = record.get(key) else {
        return Ok(None);
    };
    object(value)
}

fn event_type(event: &Fields) -> Result<Option<String>, serde_json::Error> {
    match event.get("type") {
        Some(raw) if raw.get().starts_with('"') => serde_json::from_str(raw.get()).map(Some),
        Some(_) | None => Ok(None),
    }
}

fn truthy(raw: &RawValue) -> bool {
    let text = raw.get();
    match text.as_bytes().first() {
        Some(b'[' | b'{') => text
            .get(1..)
            .is_some_and(|rest| rest.trim_start().len() > 1),
        Some(b'-' | b'0'..=b'9') => text.parse::<f64>().is_ok_and(|number| number != 0.0_f64),
        Some(_) | None => !matches!(text, "null" | "false" | "\"\""),
    }
}

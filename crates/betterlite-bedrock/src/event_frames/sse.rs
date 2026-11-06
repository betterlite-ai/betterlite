use serde::de::DeserializeOwned;

use super::error::EventFrameError;

// Mirrors types/llms/bedrock.py ServerSentEvent: missing data is "" and an empty event name is no event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSentEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<i64>,
}

impl ServerSentEvent {
    #[must_use]
    pub fn new(
        event: Option<String>,
        data: Option<String>,
        id: Option<String>,
        retry: Option<i64>,
    ) -> Self {
        Self {
            event: event.filter(|name| !name.is_empty()),
            data: data.unwrap_or_default(),
            id,
            retry,
        }
    }

    // The caller names the type the data should decode to; any JSON document is accepted, not only objects.
    pub fn json<T>(&self) -> Result<T, EventFrameError>
    where
        T: DeserializeOwned,
    {
        match serde_json::from_str(&self.data) {
            Ok(value) => Ok(value),
            Err(source) => Err(EventFrameError::NotJson(source)),
        }
    }

    // The task pins this text: values interpolated as given, with None for a missing one.
    #[must_use]
    pub fn repr(&self) -> String {
        format!(
            "ServerSentEvent(event={}, data={}, id={}, retry={})",
            self.event.as_deref().unwrap_or("None"),
            self.data,
            self.id.as_deref().unwrap_or("None"),
            self.retry
                .map_or_else(|| "None".to_owned(), |value| value.to_string()),
        )
    }
}

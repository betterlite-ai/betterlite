use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanScope {
    Full,
    LlmOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationFields {
    pub endpoint: String,
    pub headers: BTreeMap<String, String>,
    pub resource_attributes: BTreeMap<String, String>,
    pub callback_name: Option<String>,
    pub protocol: Option<String>,
    pub span_scope: SpanScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtelDestination(DestinationFields);

impl From<DestinationFields> for OtelDestination {
    fn from(fields: DestinationFields) -> Self {
        Self(fields)
    }
}

impl AsRef<DestinationFields> for OtelDestination {
    fn as_ref(&self) -> &DestinationFields {
        &self.0
    }
}

pub type CacheKey = (
    String,
    Vec<(String, String)>,
    Vec<(String, String)>,
    Option<String>,
);

impl OtelDestination {
    #[must_use]
    pub fn header_string(&self) -> String {
        self.0
            .headers
            .iter()
            .map(|(key, value)| format!("{key}={}", quote(value)))
            .collect::<Vec<_>>()
            .join(",")
    }

    #[must_use]
    pub fn cache_key(&self) -> CacheKey {
        (
            self.0.endpoint.clone(),
            self.0
                .headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            self.0
                .resource_attributes
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            self.0.protocol.clone(),
        )
    }
}

fn quote(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

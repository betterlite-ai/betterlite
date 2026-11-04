use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanScope {
    Full,
    LlmOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtelDestination {
    endpoint: String,
    headers: BTreeMap<String, String>,
    resource_attributes: BTreeMap<String, String>,
    callback_name: Option<String>,
    protocol: Option<String>,
    span_scope: SpanScope,
}

impl OtelDestination {
    #[must_use]
    pub fn new(
        endpoint: String,
        headers: BTreeMap<String, String>,
        resource_attributes: BTreeMap<String, String>,
        callback_name: Option<String>,
        protocol: Option<String>,
        span_scope: SpanScope,
    ) -> Self {
        Self {
            endpoint,
            headers,
            resource_attributes,
            callback_name,
            protocol,
            span_scope,
        }
    }

    #[must_use]
    pub fn fields(
        &self,
    ) -> (
        &str,
        &BTreeMap<String, String>,
        &BTreeMap<String, String>,
        Option<&str>,
        Option<&str>,
        SpanScope,
    ) {
        (
            &self.endpoint,
            &self.headers,
            &self.resource_attributes,
            self.callback_name.as_deref(),
            self.protocol.as_deref(),
            self.span_scope,
        )
    }

    #[must_use]
    pub fn header_string(&self) -> String {
        self.headers
            .iter()
            .map(|(key, value)| format!("{key}={}", quote(value)))
            .collect::<Vec<_>>()
            .join(",")
    }

    #[must_use]
    pub fn cache_key(
        &self,
    ) -> (
        String,
        Vec<(String, String)>,
        Vec<(String, String)>,
        Option<String>,
    ) {
        (
            self.endpoint.clone(),
            self.headers
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            self.resource_attributes
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            self.protocol.clone(),
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

use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderField {
    Text(String),
    Bytes(Vec<u8>),
}

impl HeaderField {
    fn as_text(&self) -> Cow<'_, str> {
        match self {
            Self::Text(text) => Cow::Borrowed(text),
            Self::Bytes(bytes) => Cow::Owned(bytes.iter().copied().map(char::from).collect()),
        }
    }
}

pub type HeaderPair = (HeaderField, HeaderField);

pub struct HeaderScope<'headers> {
    pub headers: &'headers [HeaderPair],
}

// LiteLLM catches every failure while reading the scope, so a scope the caller could not read is None
#[must_use]
pub fn client_sent_openai_beta_realtime_header(scope: Option<&HeaderScope<'_>>) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    scope.headers.iter().any(|(name, value)| {
        name.as_text().to_lowercase() == "openai-beta"
            && value.as_text().to_lowercase().contains("realtime=v1")
    })
}

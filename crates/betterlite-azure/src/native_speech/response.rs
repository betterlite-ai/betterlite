use std::collections::BTreeMap;

pub struct RawSpeechResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub struct HttpxBinaryResponseContent {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl From<RawSpeechResponse> for HttpxBinaryResponseContent {
    fn from(raw: RawSpeechResponse) -> Self {
        Self {
            status: raw.status,
            headers: raw.headers,
            body: raw.body,
        }
    }
}

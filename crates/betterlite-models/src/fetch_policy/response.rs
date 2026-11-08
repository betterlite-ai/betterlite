use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl FetchResponse {
    #[must_use]
    pub fn new(status: u16, headers: BTreeMap<String, String>, body: Vec<u8>) -> Self {
        let normalized = headers
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value))
            .collect();
        Self {
            status,
            headers: normalized,
            body,
        }
    }
}

use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: BTreeMap<String, Vec<String>>,
    pub body: Vec<u8>,
}

impl FetchResponse {
    #[must_use]
    pub fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, value) in headers {
            grouped
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(value);
        }
        Self {
            status,
            headers: grouped,
            body,
        }
    }

    // httpx joins repeated headers with ", " before a caller sees the value
    #[must_use]
    pub fn header(&self, name: &str) -> Option<String> {
        let values = self.headers.get(&name.to_ascii_lowercase())?;
        let joined = values.join(", ");
        Some(joined)
    }
}

mod tests;

use std::fmt;

use http::StatusCode;
use thiserror::Error;

pub const DUPLICATE_AUTHORIZATION_DETAIL: &str = "Multiple Authorization headers are not allowed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Multiple Authorization headers are not allowed")]
pub struct DuplicateAuthorization;

impl DuplicateAuthorization {
    pub const STATUS: StatusCode = StatusCode::BAD_REQUEST;
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct RawHeaders {
    entries: Vec<(String, String)>,
}

// names only: a header value can carry a bearer token or an API key.
impl fmt::Debug for RawHeaders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self
            .entries
            .iter()
            .map(|(name, _value)| name.as_str())
            .collect();
        f.debug_struct("RawHeaders").field("names", &names).finish()
    }
}

impl RawHeaders {
    pub fn from_pairs(pairs: &[(&[u8], &[u8])]) -> Result<Self, DuplicateAuthorization> {
        let authorizations = pairs
            .iter()
            .filter(|(name, _value)| name.eq_ignore_ascii_case(b"authorization"))
            .count();
        if authorizations > 1 {
            return Err(DuplicateAuthorization);
        }
        let mut collapsed: Vec<(String, String)> = Vec::with_capacity(pairs.len());
        for (raw_name, raw_value) in pairs {
            let name = latin1(raw_name);
            let value = latin1(raw_value);
            match collapsed
                .iter_mut()
                .find(|(existing, _value)| *existing == name)
            {
                Some(entry) => entry.1 = value,
                None => collapsed.push((name, value)),
            }
        }
        let entries = collapsed
            .into_iter()
            .map(|(name, value)| (name.to_lowercase(), value))
            .collect();
        Ok(Self { entries })
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&str> {
        let lowered = name.to_lowercase();
        self.entries
            .iter()
            .find(|(existing, _value)| *existing == lowered)
            .map(|(_name, value)| value.as_str())
    }

    #[must_use]
    pub fn items(&self) -> Vec<(&str, &str)> {
        self.entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect()
    }
}

fn latin1(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for byte in bytes {
        text.push(char::from(*byte));
    }
    text
}

use serde::Deserialize as _;
use serde::{Deserializer, de::Error};
pub(super) fn absolute_http_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let (scheme, authority) = value
        .split_once("://")
        .ok_or_else(|| Error::custom("url must be an absolute http(s) URL"))?;
    if !matches!(scheme, "http" | "https") || authority.is_empty() || authority.starts_with('/') {
        return Err(Error::custom("url must be an absolute http(s) URL"));
    }
    Ok(value)
}

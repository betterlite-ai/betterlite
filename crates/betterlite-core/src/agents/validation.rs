use serde::Deserialize as _;
use serde::{Deserializer, de::Error};
use url::Url;
pub(super) fn absolute_http_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let parsed = Url::parse(&value)
        .map_err(|_parse_error| Error::custom("url must be an absolute http(s) URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(Error::custom("url must be an absolute http(s) URL"));
    }
    Ok(value)
}

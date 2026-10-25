use serde::{Deserialize as _, Deserializer, de::Error};
use url::Url;

pub(super) fn absolute_http_url<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let parsed = Url::parse(&value).map_err(|parse_error| {
        Error::custom(format_args!(
            "url must be an absolute http(s) URL: {parse_error}"
        ))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(Error::custom("url must be an absolute http(s) URL"));
    }
    Ok(value)
}

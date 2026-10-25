use serde::{
    Deserialize as _, Deserializer,
    de::{Error, Visitor},
};
use std::fmt;
use std::marker::PhantomData;
use url::Url;

pub(super) fn non_null_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct NonNullOption<T>(PhantomData<T>);
    impl<'de, T> Visitor<'de> for NonNullOption<T>
    where
        T: serde::Deserialize<'de>,
    {
        type Value = Option<T>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a non-null value")
        }
        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: Error,
        {
            Err(E::custom("null is not allowed"))
        }
        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            T::deserialize(deserializer).map(Some)
        }
    }
    deserializer.deserialize_option(NonNullOption(PhantomData))
}
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

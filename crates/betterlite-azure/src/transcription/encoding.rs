use url::form_urlencoded;

// urlencode/quote_plus: a space becomes "+", and Python keeps "~" where the form serializer escapes it
#[must_use]
pub(crate) fn quote_plus(value: &str) -> String {
    let encoded = form_urlencoded::Serializer::new(String::new())
        .append_pair("value", value)
        .finish();
    encoded
        .strip_prefix("value=")
        .unwrap_or(&encoded)
        .replace('*', "%2A")
        .replace("%7E", "~")
}

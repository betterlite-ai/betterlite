use form_urlencoded::parse;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OAuthCallbackQuery {
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

// Starlette latin-1 decodes the raw query before parse_qsl, so a bare 0xFF byte stays U+00FF while %FF decodes as UTF-8
#[must_use]
pub fn decode_oauth_callback_query(query: &[u8]) -> OAuthCallbackQuery {
    let latin1: String = query.iter().map(|byte| char::from(*byte)).collect();
    let mut decoded = OAuthCallbackQuery::default();
    for (name, value) in parse(latin1.as_bytes()) {
        match name.as_ref() {
            "state" => decoded.state = Some(value.into_owned()),
            "error" => decoded.error = Some(value.into_owned()),
            "error_description" => decoded.error_description = Some(value.into_owned()),
            _ => {}
        }
    }
    decoded
}

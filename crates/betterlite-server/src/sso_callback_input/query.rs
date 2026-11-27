use form_urlencoded::parse;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OAuthCallbackQuery {
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

// Starlette's QueryParams keeps the last value of a repeated name and a blank value for a name without "="
#[must_use]
pub fn decode_oauth_callback_query(query: &[u8]) -> OAuthCallbackQuery {
    let mut decoded = OAuthCallbackQuery::default();
    for (name, value) in parse(query) {
        match name.as_ref() {
            "state" => decoded.state = Some(value.into_owned()),
            "error" => decoded.error = Some(value.into_owned()),
            "error_description" => decoded.error_description = Some(value.into_owned()),
            _ => {}
        }
    }
    decoded
}

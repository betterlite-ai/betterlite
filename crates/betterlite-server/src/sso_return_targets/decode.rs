#[must_use]
pub fn decode_sso_return_target(raw_query: &[u8]) -> Option<String> {
    let mut decoded = None;
    for (name, value) in form_urlencoded::parse(raw_query) {
        if name == "return_to" {
            decoded = Some(value.into_owned());
        }
    }
    decoded
}

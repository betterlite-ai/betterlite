#[must_use]
pub(super) fn speech_to_text_host(location: &str) -> String {
    if location == "global" {
        "speech.googleapis.com".to_owned()
    } else {
        format!("{location}-speech.googleapis.com")
    }
}

// the speech SDK dials the host itself, so a custom api_base keeps only its authority
#[must_use]
pub(super) fn api_endpoint(api_base: &str) -> String {
    let without_scheme = api_base
        .split_once("://")
        .map_or(api_base, |(_, rest)| rest);
    without_scheme
        .split_once('/')
        .map_or(without_scheme, |(authority, _)| authority)
        .to_owned()
}

use betterlite_openai::realtime::url::QueryParams;

use super::url::intent;

pub const GA_PROTOCOL: &str = "GA";
pub const BETA_PROTOCOL: &str = "beta";
pub const TRANSCRIPTION_INTENT: &str = "transcription";

#[must_use]
pub fn client_protocol(
    configured: Option<&str>,
    query_params: Option<&QueryParams>,
    client_sent_beta: bool,
) -> String {
    if let Some(protocol) = configured.filter(|value| !value.is_empty()) {
        return protocol.to_owned();
    }
    if intent(query_params) == Some(TRANSCRIPTION_INTENT) {
        return String::from(GA_PROTOCOL);
    }
    String::from(if client_sent_beta {
        BETA_PROTOCOL
    } else {
        GA_PROTOCOL
    })
}

#[must_use]
pub fn uses_beta_protocol(configured: Option<&str>) -> bool {
    configured
        .is_none_or(|value| !(value.eq_ignore_ascii_case("GA") || value.eq_ignore_ascii_case("V1")))
}

use crate::api::responses_parameters::requested::RESPONSES_OPTIONAL_PARAMS;

// the Responses optional keys B1138 declares, which LiteLLM forwards unchanged and hides from the SDK params
#[must_use]
pub fn declared_key(name: &str) -> bool {
    RESPONSES_OPTIONAL_PARAMS
        .split_whitespace()
        .any(|declared| declared == name)
}

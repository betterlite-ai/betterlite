use super::url_error::UrlError;

const ANTHROPIC_SEGMENT: &str = "/anthropic";
const MESSAGES_SUFFIX: &str = "/v1/messages";

// The AZURE_API_BASE value is looked up by the caller; litellm.api_base is never consulted
pub fn get_complete_url(
    api_base: Option<&str>,
    environment_api_base: Option<&str>,
) -> Result<String, UrlError> {
    let chosen = api_base
        .filter(|base| !base.is_empty())
        .or(environment_api_base)
        .ok_or(UrlError::MissingApiBase)?
        .trim_end_matches('/');
    if chosen.ends_with(MESSAGES_SUFFIX) {
        return Ok(chosen.to_owned());
    }
    // LiteLLM matches the first literal /anthropic anywhere in the text, query included
    let resource = chosen
        .split_once(ANTHROPIC_SEGMENT)
        .map_or(chosen, |(head, _)| head);
    Ok(format!("{resource}{ANTHROPIC_SEGMENT}{MESSAGES_SUFFIX}"))
}

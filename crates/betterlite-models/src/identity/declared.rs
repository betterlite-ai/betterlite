use crate::identity::provider_id::ProviderId;

// get_llm_provider runs an OAuth device flow for these, since their provider info carries the key it unlocks.
pub const OAUTH_VIA_PROVIDER_INFO: [ProviderId; 2] =
    [ProviderId::GithubCopilot, ProviderId::Chatgpt];

#[must_use]
pub fn authenticating_provider(
    model: Option<&str>,
    custom_llm_provider: Option<&str>,
) -> Option<ProviderId> {
    let declared_slug = if let Some(provider) = custom_llm_provider {
        provider
    } else {
        let declared_model = model?;
        let (prefix, _) = declared_model.split_once('/')?;
        prefix
    };
    let declared = ProviderId::parse(declared_slug)?;
    OAUTH_VIA_PROVIDER_INFO
        .contains(&declared)
        .then_some(declared)
}

mod tests;

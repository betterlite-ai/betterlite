use crate::loader::EnvironmentLookup;

mod tests;

pub const CLI_CLAIM_MAP_ENV: &str = "CLI_SSO_CLAIM_MAP";
pub const LITELLM_CLI_CLAIM_MAP_ENV: &str = "LITELLM_CLI_SSO_CLAIM_MAP";

// LiteLLM reads the claim map once at import: the first non-empty name wins, and an absent map is the empty map.
#[must_use]
pub fn cli_claim_map<L>(lookup: &L) -> String
where
    L: EnvironmentLookup,
{
    lookup
        .lookup(CLI_CLAIM_MAP_ENV)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            lookup
                .lookup(LITELLM_CLI_CLAIM_MAP_ENV)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_default()
}

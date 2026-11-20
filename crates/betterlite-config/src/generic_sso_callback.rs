use crate::loader::EnvironmentLookup;

mod tests;

pub const INCLUDE_TOKEN_CLAIMS_ENV: &str = "GENERIC_INCLUDE_TOKEN_CLAIMS";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenericSsoCallbackSettings {
    pub include_token_claims: bool,
}

#[must_use]
pub fn load_callback_settings<L>(lookup: &L) -> GenericSsoCallbackSettings
where
    L: EnvironmentLookup,
{
    // LiteLLM lowercases the flag and compares it with exactly "true"; any other text leaves it off
    GenericSsoCallbackSettings {
        include_token_claims: lookup
            .lookup(INCLUDE_TOKEN_CLAIMS_ENV)
            .is_some_and(|value| value.to_lowercase() == "true"),
    }
}

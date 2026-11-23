mod tests;

use betterlite_config::sections::env_credential_login::Settings;

// LiteLLM's is_env_credential_login_enabled closes env login on disable_env_credential_login alone, or on the SSO setting while a provider is fully configured.
#[must_use]
pub const fn is_enabled(settings: &Settings, sso_provider_ready: bool) -> bool {
    let closed_by_sso = settings.sso_blocks_password_login && sso_provider_ready;
    !settings.env_login_disabled && !closed_by_sso
}

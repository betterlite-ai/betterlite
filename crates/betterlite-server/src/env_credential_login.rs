mod tests;

use betterlite_config::sections::env_credential_login::Settings;

// LiteLLM's is_env_credential_login_enabled closes env login on disable_env_credential_login first, whatever SSO looks like.
#[must_use]
pub const fn is_enabled(settings: &Settings, sso_provider_ready: bool) -> bool {
    if settings.env_login_disabled {
        return false;
    }
    if settings.sso_blocks_password_login && sso_provider_ready {
        return false;
    }
    true
}

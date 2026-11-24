use secrecy::SecretString;

mod tests;

// clippy's struct_excessive_bools allows two bools per struct, and these three switches are independent
#[derive(Debug, Clone, Copy, Default)]
pub struct AuthModeSwitch(pub bool);

#[derive(Debug, Clone, Copy, Default)]
pub struct AuthModeFlags {
    pub enable_jwt_auth: AuthModeSwitch,
    pub enable_oauth2_auth: AuthModeSwitch,
    pub enable_oauth2_proxy_auth: AuthModeSwitch,
}

// an empty master key is still a key, so only an absent one leaves the dev mode reachable
#[must_use]
pub const fn is_no_auth_dev_mode(master_key: Option<&SecretString>, flags: &AuthModeFlags) -> bool {
    master_key.is_none()
        && !flags.enable_jwt_auth.0
        && !flags.enable_oauth2_auth.0
        && !flags.enable_oauth2_proxy_auth.0
}

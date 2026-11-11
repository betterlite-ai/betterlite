pub mod auth_command;
pub mod configure;
pub mod error;
pub mod logout;
pub mod print_token;
pub mod whoami;

#[must_use]
pub(crate) const fn login_command(renews: bool) -> &'static str {
    if renews {
        "lite login --pkce"
    } else {
        "lite login"
    }
}

mod tests;

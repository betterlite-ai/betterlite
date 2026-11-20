use betterlite_config::sections::key_policy_fields::AliasRule;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_secret_managers::secret_name::raise_if_unsafe_secret_name;

use crate::key_policy_fields::bad_request_for_alias;

pub const INVALID_ALIAS_MESSAGE: &str = "Invalid key_alias";

// the unsafe-name check always runs; a configured pattern or the built-in toggle adds the format rule
pub fn validate_key_alias_format(
    key_alias: Option<&str>,
    rule: Option<&AliasRule<'_>>,
) -> Result<(), Box<ProxyException>> {
    let Some(alias) = key_alias else {
        return Ok(());
    };
    if raise_if_unsafe_secret_name(alias).is_err() {
        return Err(bad_request_for_alias(INVALID_ALIAS_MESSAGE.to_owned()));
    }
    match rule {
        None => Ok(()),
        Some(selected) if selected.accepts(alias) => Ok(()),
        Some(selected) => Err(bad_request_for_alias(selected.message())),
    }
}

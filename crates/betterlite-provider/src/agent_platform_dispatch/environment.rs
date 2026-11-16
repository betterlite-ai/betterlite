use std::collections::BTreeMap;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::DispatchError;

// an environment string is read into the setting's type, and an empty one counts as unset
pub fn environment_secret(
    secrets: &SecretLookup,
    environment: &BTreeMap<String, String>,
    name: &'static str,
) -> Result<Option<SecretString>, DispatchError> {
    let value = secrets
        .get_secret_str(environment, name, None)
        .map_err(|source| DispatchError::Environment { name, source })?;
    Ok(value.filter(|text| !text.expose_secret().is_empty()))
}

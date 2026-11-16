use betterlite_models::model_registration::JsonObject;

use crate::auth_model_lists::parameters::{CREDENTIAL_FIELDS, ModelParameters};
// credential_accessor.CredentialAccessor reads this list and never writes it; B061 owns loading it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CredentialSnapshot {
    entries: Vec<CredentialEntry>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CredentialEntry {
    pub credential_name: String,
    pub credential_values: JsonObject,
}
impl CredentialSnapshot {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn new(entries: Vec<CredentialEntry>) -> Self {
        Self { entries }
    }

    // find_credential: the first entry carries the name, later duplicates never win.
    #[must_use]
    pub fn find(&self, credential_name: &str) -> Option<&CredentialEntry> {
        self.entries
            .iter()
            .find(|entry| entry.credential_name == credential_name)
    }
    // get_credential_values: a copy, so hydration can never reach the stored credential.
    #[must_use]
    pub fn values(&self, credential_name: &str) -> JsonObject {
        self.find(credential_name)
            .map_or_else(JsonObject::new, |entry| entry.credential_values.clone())
    }
}
// _hydrate_litellm_credential_name: copy on a hit, fill only declared null fields, then clear the name.
#[must_use]
pub fn hydrate_credential_name(
    parameters: Option<&ModelParameters>,
    credentials: &CredentialSnapshot,
) -> Option<ModelParameters> {
    let configured = parameters?;
    let Some(credential_name) = configured.credential_name() else {
        return Some(configured.clone());
    };
    let values = credentials.values(credential_name);
    if values.is_empty() {
        return Some(configured.clone());
    }
    let mut hydrated = configured.clone();
    for field in CREDENTIAL_FIELDS {
        if let Some(value) = values.get(field) {
            hydrated.fill_null(field, value);
        }
    }
    hydrated.clear_credential_name();
    Some(hydrated)
}

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::CredentialItem;

#[derive(Clone, Default)]
pub struct CredentialRegistry {
    entries: Arc<RwLock<Vec<CredentialItem>>>,
}

impl CredentialRegistry {
    #[must_use]
    pub fn find_credential(&self, credential_name: &str) -> Option<CredentialItem> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        let found = entries
            .iter()
            .find(|credential| credential.fields.base.credential_name == credential_name);
        found.cloned()
    }

    #[must_use]
    pub fn get_credential_values(&self, credential_name: &str) -> Payload {
        match self.find_credential(credential_name) {
            Some(credential) => credential.fields.credential_values,
            None => Payload(BTreeMap::new()),
        }
    }

    #[must_use]
    pub fn credential_list(&self) -> Vec<CredentialItem> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn upsert_credentials(&self, credentials: Vec<CredentialItem>) {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        // LiteLLM snapshots the names before the loop, so two entries new to the list both append
        let names: Vec<String> = entries
            .iter()
            .map(|credential| credential.fields.base.credential_name.clone())
            .collect();
        for credential in credentials {
            let name = credential.fields.base.credential_name.clone();
            if !names.contains(&name) {
                entries.push(credential);
            } else if let Some(slot) = entries
                .iter_mut()
                .find(|existing| existing.fields.base.credential_name == name)
            {
                *slot = credential;
            }
        }
    }

    pub fn remove_credential(&self, credential_name: &str) {
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|credential| credential.fields.base.credential_name != credential_name);
    }
}

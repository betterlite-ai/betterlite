use std::sync::{Arc, PoisonError, RwLock};

use betterlite_config::credentials::{CredentialItem, CredentialValues};

#[derive(Clone, Default)]
pub struct CredentialRegistry {
    entries: Arc<RwLock<Vec<CredentialItem>>>,
}

impl CredentialRegistry {
    #[must_use]
    pub fn find_credential(&self, credential_name: &str) -> Option<CredentialItem> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|credential| credential.credential_name == credential_name)
            .cloned()
    }

    #[must_use]
    pub fn get_credential_values(&self, credential_name: &str) -> CredentialValues {
        match self.find_credential(credential_name) {
            Some(credential) => credential.credential_values,
            None => CredentialValues::default(),
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
            .map(|credential| credential.credential_name.clone())
            .collect();
        for credential in credentials {
            let name = credential.credential_name.clone();
            if names.contains(&name) {
                if let Some(slot) = entries
                    .iter_mut()
                    .find(|existing| existing.credential_name == name)
                {
                    *slot = credential;
                }
            } else {
                entries.push(credential);
            }
        }
        drop(entries);
    }

    pub fn remove_credential(&self, credential_name: &str) {
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|credential| credential.credential_name != credential_name);
    }

    // one write lock covers lookup, merge, rename and replacement; a cold credential stays cold
    pub fn apply_credential_patch(
        &self,
        credential_name: &str,
        merged_name: &str,
        patch: &CredentialItem,
    ) {
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        let Some(resident) = entries
            .iter()
            .find(|entry| entry.credential_name == credential_name)
        else {
            return;
        };
        let mut updated = resident.clone();
        updated
            .credential_values
            .0
            .extend(patch.credential_values.0.clone());
        updated
            .credential_info
            .0
            .extend(patch.credential_info.0.clone());
        updated.credential_name = merged_name.to_owned();
        if merged_name != credential_name {
            entries.retain(|entry| entry.credential_name != credential_name);
        }
        match entries
            .iter_mut()
            .find(|entry| entry.credential_name == merged_name)
        {
            Some(slot) => *slot = updated,
            None => entries.push(updated),
        }
        drop(entries);
    }
}

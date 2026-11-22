use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::error::Error;

use betterlite_config::sections::key_event_sinks::Settings;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_secret_managers::secret_contracts::BaseSecretManager;
use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretWrite,
};
use secrecy::{ExposeSecret, SecretString};
use tracing::{debug, info, warn};

use super::error::{SecretSinkError, SinkError};
use super::ports::TeamDirectory;
use super::team_params::{TeamSecretSettings, secret_manager_settings};

pub struct SecretSink<'deps, Manager> {
    pub settings: &'deps Settings,
    // litellm.secret_manager_client when it is a BaseSecretManager; any other client stores nothing
    pub manager: Option<&'deps Manager>,
    // proxy_server's database and key cache; None when the proxy runtime is absent
    pub teams: Option<&'deps dyn TeamDirectory>,
}

impl<Manager> SecretSink<'_, Manager>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + 'static,
{
    pub async fn store(
        &self,
        secret_name: &str,
        secret_token: &SecretString,
        team_id: Option<&str>,
    ) -> Result<(), SinkError<Manager>> {
        let Some(manager) = self.manager.filter(|_| self.settings.store_virtual_keys) else {
            return Ok(());
        };
        let tags = self.settings.tags.as_ref();
        let description = self.settings.description.as_deref();
        let params = self.params(team_id).await?;
        debug!(
            "Creating secret with {secret_name} and tags={tags:?} and description={description:?}"
        );
        let name = self.secret_name(secret_name);
        let write = SecretWrite {
            secret_name: &name,
            secret_value: secret_token,
            description,
            options: SecretOptions {
                optional_params: params.as_ref(),
                timeout: None,
            },
            tags,
        };
        manager
            .async_write_secret(write)
            .await
            .map_err(|source| SecretSinkError::Manager { source })?;
        Ok(())
    }

    pub async fn rotate(
        &self,
        current_secret_name: &str,
        new_secret_name: &str,
        new_secret_value: &SecretString,
        team_id: Option<&str>,
    ) -> Result<(), SinkError<Manager>> {
        let Some(manager) = self.manager.filter(|_| self.settings.store_virtual_keys) else {
            return Ok(());
        };
        let params = self.params(team_id).await?;
        let rotation = SecretRotation {
            current_secret_name,
            new_secret_name,
            new_secret_value,
            options: SecretOptions {
                optional_params: params.as_ref(),
                timeout: None,
            },
        };
        self.rotate_prefixed(manager, rotation).await
    }

    // an empty value read back is still a secret; only a missing one skips the move
    pub async fn rename(
        &self,
        current_secret_name: &str,
        new_secret_name: &str,
        team_id: Option<&str>,
    ) -> Result<(), SinkError<Manager>> {
        let Some(manager) = self.manager.filter(|_| self.settings.store_virtual_keys) else {
            return Ok(());
        };
        let params = self.params(team_id).await?;
        let options = SecretOptions {
            optional_params: params.as_ref(),
            timeout: None,
        };
        let current_value = manager
            .async_read_secret(&self.secret_name(current_secret_name), options)
            .await
            .map_err(|source| SecretSinkError::Manager { source })?;
        let Some(value) = current_value else {
            warn!(
                "Secret {current_secret_name} not found in secret manager, skipping rename to {new_secret_name}"
            );
            return Ok(());
        };
        info!(
            "Renaming secret in secret manager: current_secret_name={current_secret_name} new_secret_name={new_secret_name}"
        );
        let rotation = SecretRotation {
            current_secret_name,
            new_secret_name,
            new_secret_value: &value,
            options,
        };
        self.rotate_prefixed(manager, rotation).await
    }

    // one delete after another, each team's params looked up once, and the first failure stops the rest
    pub async fn delete(
        &self,
        keys_being_deleted: &[LiteLLMVerificationToken],
    ) -> Result<(), SinkError<Manager>> {
        let Some(manager) = self.manager.filter(|_| self.settings.store_virtual_keys) else {
            return Ok(());
        };
        let mut team_params = BTreeMap::new();
        for key in keys_being_deleted {
            let Some(alias) = &key.key_alias else {
                let token = key
                    .token
                    .as_ref()
                    .map_or("None", ExposeSecret::expose_secret);
                warn!(
                    "Key alias not found for key {token}. Skipping deletion from secret manager."
                );
                continue;
            };
            let params = match team_params.entry(key.team_id.clone()) {
                Entry::Occupied(cached) => cached.into_mut(),
                Entry::Vacant(slot) => slot.insert(self.params(key.team_id.as_deref()).await?),
            };
            let options = DeleteOptions {
                recovery_window_in_days: 7,
                operation: SecretOptions {
                    optional_params: params.as_ref(),
                    timeout: None,
                },
            };
            manager
                .async_delete_secret(&self.secret_name(alias), options)
                .await
                .map_err(|source| SecretSinkError::Manager { source })?;
        }
        Ok(())
    }

    async fn rotate_prefixed(
        &self,
        manager: &Manager,
        rotation: SecretRotation<'_, Manager::Params>,
    ) -> Result<(), SinkError<Manager>> {
        let current_secret_name = self.secret_name(rotation.current_secret_name);
        let new_secret_name = self.secret_name(rotation.new_secret_name);
        let prefixed = SecretRotation {
            current_secret_name: &current_secret_name,
            new_secret_name: &new_secret_name,
            new_secret_value: rotation.new_secret_value,
            options: rotation.options,
        };
        manager
            .async_rotate_secret(prefixed)
            .await
            .map_err(|source| SecretSinkError::Rotation { source })?;
        Ok(())
    }

    // the team settings become the manager's own params after the lookup, so a misfit propagates
    async fn params(
        &self,
        team_id: Option<&str>,
    ) -> Result<Option<Manager::Params>, SinkError<Manager>> {
        secret_manager_settings(self.teams, team_id)
            .await
            .map(<Manager::Params as TryFrom<TeamSecretSettings>>::try_from)
            .transpose()
            .map_err(|source| SecretSinkError::Params { source })
    }

    fn secret_name(&self, secret_name: &str) -> String {
        if self.settings.prefix_for_stored_virtual_keys.ends_with('/') {
            format!(
                "{}{secret_name}",
                self.settings.prefix_for_stored_virtual_keys
            )
        } else {
            format!(
                "{}/{secret_name}",
                self.settings.prefix_for_stored_virtual_keys
            )
        }
    }
}

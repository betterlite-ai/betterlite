use std::collections::{BTreeMap, BTreeSet};
use std::future;
use std::sync::Arc;

use betterlite_secret_managers::secret_contracts::options::{
    DeleteOptions, SecretOptions, SecretRotation, SecretWrite,
};
use betterlite_secret_managers::secret_contracts::{
    BaseSecretManager, RotationError, SecretFuture,
};
use betterlite_server::key_event_sinks::team_params::TeamSecretSettings;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::board::Board;

#[derive(Debug, thiserror::Error)]
#[error("the scripted secret manager refused {name}")]
pub(crate) struct Refused {
    name: String,
}

pub(crate) struct ScriptedManager {
    pub(crate) board: Arc<Board>,
    pub(crate) secrets: BTreeMap<String, String>,
    pub(crate) failing: BTreeSet<String>,
}

impl ScriptedManager {
    fn answer(&self, effect: Value, name: &str) -> Result<(), Refused> {
        self.board.record(effect);
        if self.failing.contains(name) {
            return Err(Refused {
                name: name.to_owned(),
            });
        }
        Ok(())
    }
}

impl BaseSecretManager for ScriptedManager {
    type Params = TeamSecretSettings;
    type Response = ();
    type Error = Refused;

    fn async_read_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: SecretOptions<'borrowed, TeamSecretSettings>,
    ) -> SecretFuture<'borrowed, Option<SecretString>, Refused> {
        Box::pin(future::ready(self.sync_read_secret(secret_name, options)))
    }

    fn sync_read_secret(
        &self,
        secret_name: &str,
        options: SecretOptions<'_, TeamSecretSettings>,
    ) -> Result<Option<SecretString>, Refused> {
        self.answer(
            json!(["read", secret_name, options.optional_params]),
            secret_name,
        )?;
        Ok(self
            .secrets
            .get(secret_name)
            .map(|value| SecretString::from(value.as_str())))
    }

    fn async_write_secret(
        &self,
        write: SecretWrite<'_, TeamSecretSettings>,
    ) -> SecretFuture<'_, (), Refused> {
        let value = write.secret_value.expose_secret();
        let effect = json!([
            "write",
            write.secret_name,
            value,
            write.description,
            write.tags,
            write.options.optional_params
        ]);
        Box::pin(future::ready(self.answer(effect, write.secret_name)))
    }

    fn async_delete_secret<'borrowed>(
        &'borrowed self,
        secret_name: &'borrowed str,
        options: DeleteOptions<'borrowed, TeamSecretSettings>,
    ) -> SecretFuture<'borrowed, (), Refused> {
        let effect = json!([
            "delete",
            secret_name,
            options.recovery_window_in_days,
            options.operation.optional_params
        ]);
        Box::pin(future::ready(self.answer(effect, secret_name)))
    }

    fn async_rotate_secret(
        &self,
        rotation: SecretRotation<'_, TeamSecretSettings>,
    ) -> SecretFuture<'_, (), RotationError<Refused>> {
        let value = rotation.new_secret_value.expose_secret();
        let effect = json!([
            "rotate",
            rotation.current_secret_name,
            rotation.new_secret_name,
            value,
            rotation.options.optional_params
        ]);
        let answer = self.answer(effect, rotation.current_secret_name);
        Box::pin(future::ready(
            answer.map_err(|source| RotationError::Operation { source }),
        ))
    }
}

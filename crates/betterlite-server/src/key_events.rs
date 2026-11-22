pub mod audit;
pub mod deleted;
pub mod effects;
pub mod encode;
pub mod error;
pub mod generated;
pub mod identifiers;
pub mod rotated;
pub mod types;
pub mod updated;

mod tests;

use std::error::Error;

use betterlite_secret_managers::secret_contracts::BaseSecretManager;

use self::audit::KeyEventAudit;
use self::effects::KeyEventEffects;
use self::error::KeyEventError;
use self::types::{KeyEvent, KeyEventCaller};
use crate::key_event_sinks::team_params::TeamSecretSettings;

// one proxy-facing boundary over the four key lifecycle hooks; each hook schedules its audit rows and never awaits them, so a later sink failure cannot undo the committed key change
pub struct KeyEvents<'deps, Manager> {
    pub audit: &'deps KeyEventAudit,
    pub effects: KeyEventEffects<'deps, Manager>,
}

impl<Manager> KeyEvents<'_, Manager>
where
    Manager: BaseSecretManager,
    Manager::Params: TryFrom<TeamSecretSettings> + Send,
    <Manager::Params as TryFrom<TeamSecretSettings>>::Error: Error + Send + Sync + 'static,
{
    // KeyManagementEventHooks.async_key_{generated,updated,rotated,deleted}_hook
    pub async fn dispatch(
        &self,
        event: &KeyEvent,
        caller: &KeyEventCaller,
        litellm_changed_by: Option<&str>,
    ) -> Result<(), KeyEventError> {
        match event {
            KeyEvent::Generated(event) => {
                generated::run(&self.effects, self.audit, event, caller, litellm_changed_by).await
            }
            KeyEvent::Updated(event) => {
                updated::run(&self.effects, self.audit, event, caller, litellm_changed_by).await
            }
            KeyEvent::Rotated(event) => {
                rotated::run(&self.effects, self.audit, event, caller, litellm_changed_by).await
            }
            KeyEvent::Deleted(event) => {
                deleted::run(&self.effects, self.audit, event, caller, litellm_changed_by).await
            }
        }
    }
}

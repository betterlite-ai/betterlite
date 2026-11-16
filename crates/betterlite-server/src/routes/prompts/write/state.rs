use std::sync::Arc;

use betterlite_auth::auth_identity::roles::Role;
use betterlite_prompts::registry::mutation::prompt_registry_mutation::callbacks::PromptCallbackManager;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::PromptTableActions;
use betterlite_prompts::registry::prompt_registry::InMemoryPromptRegistry;
use tokio::sync::Mutex;

use super::error::PromptWriteError;
use crate::auth::auth_identity::AuthContext;

const MASTER_KEY_SUBJECT: &str = "master_key";
// LiteLLM names the master key's user this way, and the name lands in created_by
const MASTER_KEY_USER_ID: &str = "default_user_id";

pub(crate) struct PromptWriteState<Table> {
    pub(crate) table: Option<Table>,
    pub(crate) registry: Arc<Mutex<InMemoryPromptRegistry>>,
    pub(crate) callbacks: Arc<Mutex<dyn PromptCallbackManager + Send>>,
}

pub(super) struct Writer<'state, Table> {
    pub(super) table: &'state Table,
    pub(super) user_id: Option<String>,
}

impl<Table> PromptWriteState<Table>
where
    Table: PromptTableActions,
{
    pub(super) fn require_admin_and_db(
        &self,
        context: &AuthContext,
        action: &'static str,
    ) -> Result<Writer<'_, Table>, PromptWriteError<Table::Failure>> {
        let principal = &context.principal;
        let is_master_key = principal.subject == MASTER_KEY_SUBJECT;
        if !is_master_key && !principal.roles.contains(&Role::PlatformAdmin) {
            return Err(PromptWriteError::Forbidden { action });
        }
        let table = self
            .table
            .as_ref()
            .ok_or(PromptWriteError::DbNotConnected)?;
        let user_id = principal
            .user
            .as_ref()
            .map(|user| user.id.clone())
            .or_else(|| is_master_key.then(|| MASTER_KEY_USER_ID.to_owned()));
        Ok(Writer { table, user_id })
    }
}

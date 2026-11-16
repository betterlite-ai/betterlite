mod tests;

use std::sync::PoisonError;

use betterlite_policy::records::attachment::PolicyAttachmentDBResponse;
use betterlite_policy::values::PolicyAttachment;

use super::attachment_list::get_all_attachments_from_db;
use super::attachment_row::AttachmentTableSource;
use super::error::Operation;
use super::{AttachmentError, AttachmentStore};

pub async fn sync_attachments_from_db<Client>(
    store: &AttachmentStore<'_, Client>,
) -> Result<(), AttachmentError<Client>>
where
    Client: AttachmentTableSource,
{
    let stored = get_all_attachments_from_db(store)
        .await
        .map_err(|error| error.within(Operation::SyncAttachments).logged())?;
    let stored_count = stored.len();
    let db_attachments = stored.into_iter().map(registered_attachment);
    let config_count = {
        let mut registry = store
            .registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut attachments = registry.config_attachments.clone();
        attachments.extend(db_attachments);
        registry.attachments = attachments;
        registry.initialized = true;
        registry.config_attachments.len()
    };
    tracing::info!(
        "Synced {stored_count} attachments from DB to in-memory registry ({config_count} config-defined attachments preserved)"
    );
    Ok(())
}

// the table stores an unset scope list as [], which the in-memory attachment spells None
pub(super) fn registered_attachment(response: PolicyAttachmentDBResponse) -> PolicyAttachment {
    PolicyAttachment {
        policy: response.policy_name,
        scope: response.scope,
        teams: Some(response.teams).filter(|teams| !teams.is_empty()),
        keys: Some(response.keys).filter(|keys| !keys.is_empty()),
        models: Some(response.models).filter(|models| !models.is_empty()),
        tags: Some(response.tags).filter(|tags| !tags.is_empty()),
        priority: response.priority,
        default: response.default,
    }
}

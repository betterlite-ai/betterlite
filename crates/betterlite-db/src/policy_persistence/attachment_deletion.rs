mod tests;

use betterlite_policy::pipeline::json_value::JsonValue;
use betterlite_policy::row_adapter::{PolicyClause, PolicyTable as _};

use super::attachment_row::AttachmentTableSource;
use super::attachment_sync::sync_attachments_from_db;
use super::error::Operation;
use super::{AttachmentError, AttachmentStore, Deletion};

pub async fn delete_attachment_from_db<Client>(
    store: &AttachmentStore<'_, Client>,
    attachment_id: &str,
) -> Result<Deletion, AttachmentError<Client>>
where
    Client: AttachmentTableSource,
{
    let chain = || vec![Operation::DeleteAttachment];
    let failed = |source| AttachmentError::<Client>::Database(chain(), source).logged();
    let table = store.client.attachment_table();
    let filter = PolicyClause::from([(
        "attachment_id".to_owned(),
        JsonValue::String(attachment_id.to_owned()),
    )]);
    if table
        .find_unique(filter.clone())
        .await
        .map_err(failed)?
        .is_none()
    {
        return Err(AttachmentError::<Client>::AttachmentNotFound(
            chain(),
            attachment_id.to_owned(),
        )
        .logged());
    }
    table.delete(filter).await.map_err(failed)?;
    // in-memory attachments carry no ID, so the list is rebuilt from the table
    sync_attachments_from_db(store)
        .await
        .map_err(|error| error.within(Operation::DeleteAttachment).logged())?;
    Ok(Deletion {
        message: format!("Attachment {attachment_id} deleted successfully"),
        warning: None,
    })
}

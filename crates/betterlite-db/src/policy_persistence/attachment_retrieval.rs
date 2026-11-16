mod tests;

use betterlite_policy::pipeline::json_value::JsonValue;
use betterlite_policy::records::attachment::PolicyAttachmentDBResponse;
use betterlite_policy::row_adapter::{PolicyClause, PolicyTable as _};

use super::attachment_row::AttachmentTableSource;
use super::error::Operation;
use super::{AttachmentError, AttachmentStore};

pub async fn get_attachment_by_id_from_db<Client>(
    store: &AttachmentStore<'_, Client>,
    attachment_id: &str,
) -> Result<Option<PolicyAttachmentDBResponse>, AttachmentError<Client>>
where
    Client: AttachmentTableSource,
{
    let filter = PolicyClause::from([(
        "attachment_id".to_owned(),
        JsonValue::String(attachment_id.to_owned()),
    )]);
    let found = store
        .client
        .attachment_table()
        .find_unique(filter)
        .await
        .map_err(|source| {
            AttachmentError::<Client>::Database(vec![Operation::GetAttachment], source).logged()
        })?;
    Ok(found.map(PolicyAttachmentDBResponse::from))
}

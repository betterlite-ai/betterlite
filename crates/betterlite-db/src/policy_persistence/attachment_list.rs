mod tests;

use betterlite_policy::records::attachment::PolicyAttachmentDBResponse;
use betterlite_policy::row_adapter::{PolicyOrder, PolicySort, PolicyTable as _};

use super::attachment_row::AttachmentTableSource;
use super::error::Operation;
use super::{AttachmentError, AttachmentStore};

pub async fn get_all_attachments_from_db<Client>(
    store: &AttachmentStore<'_, Client>,
) -> Result<Vec<PolicyAttachmentDBResponse>, AttachmentError<Client>>
where
    Client: AttachmentTableSource,
{
    let order = PolicyOrder::from([("created_at".to_owned(), PolicySort::Desc)]);
    let rows = store
        .client
        .attachment_table()
        .find_many(None, Some(order))
        .await
        .map_err(|source| {
            AttachmentError::<Client>::Database(vec![Operation::ListAttachments], source).logged()
        })?;
    Ok(rows
        .into_iter()
        .map(PolicyAttachmentDBResponse::from)
        .collect())
}

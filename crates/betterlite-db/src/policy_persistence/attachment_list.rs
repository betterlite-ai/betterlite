mod tests;

use betterlite_policy::records::attachment::PolicyAttachmentDBResponse;
use betterlite_policy::row_adapter::{PolicyOrder, PolicySort, PolicyTable as _};

use super::attachment_row::AttachmentTableSource;
use super::error::{Operation, OperationChain};
use super::{AttachmentError, AttachmentStore};

pub async fn get_all_attachments_from_db<Client: AttachmentTableSource>(
    store: &AttachmentStore<'_, Client>,
) -> Result<Vec<PolicyAttachmentDBResponse>, AttachmentError<Client>> {
    let order = PolicyOrder::from([("created_at".to_owned(), PolicySort::Desc)]);
    let rows = store
        .client
        .attachment_table()
        .find_many(None, Some(order))
        .await
        .map_err(|source| {
            AttachmentError::<Client>::Database(
                OperationChain::from(Operation::ListAttachments),
                source,
            )
            .logged()
        })?;
    Ok(rows
        .into_iter()
        .map(PolicyAttachmentDBResponse::from)
        .collect())
}

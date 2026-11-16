mod tests;

use std::sync::PoisonError;

use betterlite_policy::records::attachment::{
    PolicyAttachmentCreateRequest, PolicyAttachmentDBResponse,
};
use betterlite_policy::row_adapter::{PolicyClause, PolicyTable as _};
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::attachment_row::AttachmentTableSource;
use super::attachment_sync::registered_attachment;
use super::error::Operation;
use super::{AttachmentError, AttachmentStore};

#[derive(Serialize)]
struct NewAttachment<'request> {
    policy_name: &'request str,
    scope: Option<&'request str>,
    teams: &'request [String],
    keys: &'request [String],
    models: &'request [String],
    tags: &'request [String],
    priority: Option<i32>,
    is_default: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    created_by: Option<&'request str>,
    updated_by: Option<&'request str>,
}

pub async fn add_attachment_to_db<Client>(
    store: &AttachmentStore<'_, Client>,
    request: &PolicyAttachmentCreateRequest,
    created_by: Option<&str>,
) -> Result<PolicyAttachmentDBResponse, AttachmentError<Client>>
where
    Client: AttachmentTableSource,
{
    let chain = || vec![Operation::AddAttachment];
    let now = (store.now)();
    let row = NewAttachment {
        policy_name: &request.policy_name,
        scope: request.scope.as_deref(),
        teams: request.teams.as_deref().unwrap_or_default(),
        keys: request.keys.as_deref().unwrap_or_default(),
        models: request.models.as_deref().unwrap_or_default(),
        tags: request.tags.as_deref().unwrap_or_default(),
        priority: request.priority,
        is_default: request.default,
        created_at: now,
        updated_at: now,
        created_by,
        updated_by: created_by,
    };
    let data = serde_json::to_string(&row)
        .and_then(|text| serde_json::from_str::<PolicyClause>(&text))
        .map_err(|source| AttachmentError::<Client>::Json(chain(), source).logged())?;
    let table = store.client.attachment_table();
    let created = table
        .create(data)
        .await
        .map_err(|source| AttachmentError::<Client>::Database(chain(), source).logged())?;
    let response = PolicyAttachmentDBResponse::from(created);
    // the stored row holds the request's values, so registering it matches a later resync
    let attachment = registered_attachment(response.clone());
    store
        .registry
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .add_attachment(attachment);
    Ok(response)
}

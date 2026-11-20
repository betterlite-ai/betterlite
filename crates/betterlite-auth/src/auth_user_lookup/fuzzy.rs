use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;

use super::lookup::{BlockFailure, UserLookup};
use super::store::UserStore;

// _get_fuzzy_user_object: an exact SSO id first, then a case-insensitive email, never a create
pub(super) async fn fuzzy_user(
    lookup: &UserLookup,
    store: &dyn UserStore,
    sso_user_id: Option<&str>,
    user_email: Option<&str>,
) -> Result<Option<JsonData>, BlockFailure> {
    let mut response = match sso_user_id {
        Some(sso) => store
            .find_by_sso_id(sso)
            .await
            .map_err(|error| BlockFailure::Store(Box::new(error)))?,
        None => None,
    };
    if response.is_none()
        && let Some(email) = user_email
    {
        response = store
            .find_by_email(email)
            .await
            .map_err(|error| BlockFailure::Store(Box::new(error)))?;
        if let Some(record) = response.as_ref()
            && let Some(sso) = sso_user_id
        {
            schedule_sso_update(lookup, record, sso);
        }
    }
    Ok(response)
}

// asyncio.create_task(...update sso_user_id...): the request never waits for this write
fn schedule_sso_update(lookup: &UserLookup, record: &JsonData, sso_user_id: &str) {
    let Some(store) = lookup.store.clone() else {
        return;
    };
    let Some(user_id) = record_user_id(record) else {
        return;
    };
    let sso_user_id = sso_user_id.to_owned();
    let redactor = Arc::clone(&lookup.redactor);
    lookup.tasks.spawn(async move {
        match store.update_sso_user_id(&user_id, &sso_user_id).await {
            Ok(()) => (),
            Err(error) => {
                let cause = redactor.redact_internal_details(&error.to_string());
                tracing::warn!(cause = %cause, "the background SSO user id update failed");
            }
        }
    });
}

fn record_user_id(record: &JsonData) -> Option<String> {
    let JsonData::Object(fields) = record else {
        return None;
    };
    match fields.get("user_id") {
        Some(JsonData::String(user_id)) => Some(user_id.clone()),
        None | Some(_) => None,
    }
}

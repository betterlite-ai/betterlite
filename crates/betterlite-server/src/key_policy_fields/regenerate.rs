use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use crate::key_fields::decode::{DecodeError, TypedValue};
use crate::key_fields::regenerate::RegenerateKeyRequest;
use crate::key_fields::update::UpdateKeyRequest;

// the request fields a regenerate request carries where UpdateKeyRequest declares no such field
const NON_UPDATE_FIELDS: [&str; 5] = [
    "grace_period",
    "key_type",
    "new_key",
    "new_master_key",
    "send_invite_email",
];
const EMPTY_DURATIONS: [&str; 2] = ["duration", "budget_duration"];

// an empty patch is no update at all; otherwise the patch validates as the UpdateKeyRequest it becomes
pub fn regenerate_as_update_request(
    key: &str,
    data: &RegenerateKeyRequest,
) -> Result<Option<UpdateKeyRequest>, DecodeError> {
    let supplied = data.project()?.supplied;
    let mut patch = BTreeMap::new();
    for (field, value) in supplied {
        if field == "key" || NON_UPDATE_FIELDS.contains(&field.as_str()) {
            continue;
        }
        let empty_duration =
            EMPTY_DURATIONS.contains(&field.as_str()) && value == JsonData::String(String::new());
        if empty_duration {
            continue;
        }
        patch.insert(field, value);
    }
    if patch.is_empty() {
        return Ok(None);
    }
    patch.insert("key".to_owned(), JsonData::String(key.to_owned()));
    let request = UpdateKeyRequest::read(&TypedValue {
        json: JsonData::Object(patch),
    })?;
    Ok(Some(request))
}

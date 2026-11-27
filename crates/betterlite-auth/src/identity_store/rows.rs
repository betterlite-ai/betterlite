use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::combined_key_read::view::CombinedKeyView;

use crate::auth_key_lookup::error::KeyLookupError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

// UserAPIKeyAuth.model_validate(fields) over a row dumped with exclude_none=True.
pub(super) fn auth_from_fields(
    mut fields: BTreeMap<String, JsonData>,
) -> Result<UserAPIKeyAuth, KeyLookupError> {
    fields.retain(|_, value| !matches!(value, JsonData::Null));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
        .map_err(|error| KeyLookupError::Row(Box::new(error)))
}

// UserAPIKeyAuth.model_validate(row.model_dump(exclude_none=True)): a null row field reads as its default.
pub(super) fn auth_from_view(view: &CombinedKeyView) -> Result<UserAPIKeyAuth, KeyLookupError> {
    let text = serde_json::to_string(view).map_err(KeyLookupError::Json)?;
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(&text).map_err(KeyLookupError::Json)?;
    auth_from_fields(fields)
}

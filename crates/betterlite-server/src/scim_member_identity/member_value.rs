use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::error::ScimMemberIdentityError;
use crate::scim_projections::truthiness::is_set;
use crate::scim_records::group::Member;
use crate::scim_records::{MANAGED_TEAM_METADATA_KEY, TEAM_DATA_METADATA_KEY};

// str.strip()'s whitespace set also covers U+001C-U+001F; Rust's is_whitespace() does not
pub(crate) const fn is_strip_whitespace(character: char) -> bool {
    match character {
        '\u{1c}'..='\u{1f}' => true,
        _ => character.is_whitespace(),
    }
}

// an empty member id is a client error, while a nonempty one is looked up in its original spelling
pub fn member_value(member: &Member) -> Result<&str, ScimMemberIdentityError> {
    let value = member.value.as_str();
    if value.trim_matches(is_strip_whitespace).is_empty() {
        return Err(ScimMemberIdentityError::EmptyMemberValue);
    }
    Ok(value)
}

pub fn normalized_member_type(member: &Member) -> Option<String> {
    let declared = member.kind.0.as_deref().unwrap_or_default();
    let normalized = declared.trim_matches(is_strip_whitespace).to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized)
}

#[must_use]
pub const fn json_object_fields(raw: &JsonData) -> Option<&BTreeMap<String, JsonData>> {
    match raw {
        JsonData::Object(fields) => Some(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => None,
    }
}

// a working LiteLLM setup reads `bool(metadata.scim_managed)` and `metadata.scim_data is not None`
#[must_use]
pub fn metadata_has_scim_provenance(fields: &BTreeMap<String, JsonData>) -> bool {
    let managed = fields.get(MANAGED_TEAM_METADATA_KEY).is_some_and(is_set);
    let data = fields
        .get(TEAM_DATA_METADATA_KEY)
        .is_some_and(|value| !matches!(value, JsonData::Null));
    managed || data
}

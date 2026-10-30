use super::decoder::{TeamRowError, parse_data};
use crate::budget_permission_rows::JsonData;

pub fn set_model_info(value: &mut JsonData) -> Result<(), TeamRowError> {
    let JsonData::Object(object) = value else {
        return Err(TeamRowError::Validation("input needs to be a dictionary"));
    };
    if matches!(object.get("members_with_roles"), Some(JsonData::Object(fields)) if fields.is_empty())
    {
        object.insert("members_with_roles".to_owned(), JsonData::Array(Vec::new()));
    }
    for field in [
        "metadata",
        "aliases",
        "config",
        "permissions",
        "model_max_budget",
        "model_aliases",
        "router_settings",
        "budget_limits",
    ] {
        if let Some(JsonData::String(text)) = object.get(field) {
            let parsed = parse_data(text.as_bytes()).map_err(|error| match error {
                TeamRowError::Json(source) => TeamRowError::Field { field, source },
                other @ (TeamRowError::TooLarge { .. }
                | TeamRowError::InvalidUtf8(_)
                | TeamRowError::TooDeep
                | TeamRowError::Validation(_)
                | TeamRowError::Field { .. }) => other,
            })?;
            object.insert(field.to_owned(), parsed);
        }
    }
    Ok(())
}

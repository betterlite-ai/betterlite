use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// A model_info text field: an absent, null or non-text value reads as none.
pub(crate) fn text<'info>(model_info: &'info JsonObject, field: &str) -> Option<&'info str> {
    match model_info.get(field) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}

// The two team fields of a deployment's model_info, read once per row.
pub(crate) struct TeamFields<'info> {
    pub(crate) owner: Option<&'info str>,
    pub(crate) public_name: Option<&'info str>,
}

impl<'info> TeamFields<'info> {
    #[must_use]
    pub(crate) fn read(model_info: &'info JsonObject) -> Self {
        Self {
            owner: text(model_info, "team_id"),
            public_name: text(model_info, "team_public_model_name"),
        }
    }
}

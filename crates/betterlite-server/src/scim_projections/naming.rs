use super::directory::Metadata;
use super::error::ProjectionError;
use crate::scim_records::user_metadata::UserScimMetadata;
use betterlite_db::budget_permission_rows::JsonData;

pub const DEFAULT_SCIM_NAME: &str = "Unknown User";
pub const DEFAULT_SCIM_FAMILY_NAME: &str = "Unknown Family Name";
pub const DEFAULT_SCIM_DISPLAY_NAME: &str = "Unknown Display Name";
pub const DEFAULT_SCIM_MEMBER_VALUE: &str = "Unknown Member Value";
const SCIM_METADATA_KEY: &str = "scim_metadata";

pub struct ScimNames {
    pub user_name: String,
    pub given_name: String,
    pub family_name: String,
}

// key presence alone decides whether scim_metadata is read, so a stored non-object fails instead of falling back
fn stored_names(metadata: &Metadata) -> Result<Option<UserScimMetadata>, ProjectionError> {
    match metadata.get(SCIM_METADATA_KEY) {
        None => Ok(None),
        Some(object @ JsonData::Object(_)) => {
            let text = serde_json::to_string(object)
                .map_err(|source| ProjectionError::Encode { source })?;
            let stored = serde_json::from_str(&text)
                .map_err(|source| ProjectionError::ScimMetadata { source })?;
            Ok(Some(stored))
        }
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_),
        ) => Err(ProjectionError::ScimMetadataShape),
    }
}

pub fn scim_names(
    user_email: Option<&str>,
    user_alias: Option<&str>,
    metadata: &Metadata,
) -> Result<ScimNames, ProjectionError> {
    let non_empty = |text: &&str| !text.is_empty();
    let alias = user_alias.filter(non_empty);
    let stored = stored_names(metadata)?.unwrap_or(UserScimMetadata {
        given_name: None,
        family_name: None,
    });
    Ok(ScimNames {
        user_name: user_email
            .filter(non_empty)
            .unwrap_or(DEFAULT_SCIM_DISPLAY_NAME)
            .to_owned(),
        given_name: stored
            .given_name
            .as_deref()
            .filter(non_empty)
            .or(alias)
            .unwrap_or(DEFAULT_SCIM_NAME)
            .to_owned(),
        family_name: stored
            .family_name
            .as_deref()
            .filter(non_empty)
            .or(alias)
            .unwrap_or(DEFAULT_SCIM_FAMILY_NAME)
            .to_owned(),
    })
}

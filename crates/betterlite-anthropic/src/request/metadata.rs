use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::option_primitives::user_id::{UserIdError, UserIdPolicy};

const METADATA: &str = "metadata";
const USER_ID: &str = "user_id";

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("metadata.user_id must be a string")]
    UserIdNotText,
    #[error(transparent)]
    UserId(#[from] UserIdError),
}

impl MetadataError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UserIdNotText => Some(400),
            Self::UserId(UserIdError::Pattern(_)) => None,
        }
    }
}

// The control-plane metadata supplies metadata.user_id only when it names a person without contact details
pub(super) fn apply_user_id(
    params: &mut IndexMap<String, Json>,
    control: Option<&Json>,
) -> Result<(), MetadataError> {
    if let Some(Json::Object(fields)) = control
        && !fields.is_empty()
    {
        apply_control_user_id(params, fields)?;
    }
    reduce_metadata(params);
    Ok(())
}

fn apply_control_user_id(
    params: &mut IndexMap<String, Json>,
    control: &IndexMap<String, Json>,
) -> Result<(), MetadataError> {
    let Some(value) = control.get(USER_ID) else {
        return Ok(());
    };
    let Json::Text(user_id) = value else {
        return invalid_control_user_id(value);
    };
    if !UserIdPolicy::compile()?.accepts(user_id) {
        return Ok(());
    }
    let mut metadata = IndexMap::new();
    metadata.insert(USER_ID.to_owned(), Json::Text(user_id.clone()));
    params.insert(METADATA.to_owned(), Json::Object(metadata));
    Ok(())
}

// Python's re.match raises TypeError for a value that is neither null nor text, and the request fails there
const fn invalid_control_user_id(value: &Json) -> Result<(), MetadataError> {
    match value {
        Json::Null => Ok(()),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(MetadataError::UserIdNotText)
        }
    }
}

// A preexisting option metadata.user_id is kept as sent; every other metadata key is not part of the API
fn reduce_metadata(params: &mut IndexMap<String, Json>) {
    let Some(Json::Object(fields)) = params.get(METADATA) else {
        return;
    };
    let kept = fields
        .get(USER_ID)
        .filter(|value| !matches!(value, Json::Null))
        .cloned();
    match kept {
        None => {
            params.shift_remove(METADATA);
        }
        Some(user_id) => {
            let mut metadata = IndexMap::new();
            metadata.insert(USER_ID.to_owned(), user_id);
            params.insert(METADATA.to_owned(), Json::Object(metadata));
        }
    }
}

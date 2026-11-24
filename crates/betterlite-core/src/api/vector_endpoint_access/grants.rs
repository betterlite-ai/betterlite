use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::identity_rows::Payload;
use serde::Deserialize;

use super::error::VectorEndpointAccessError;

// the key and team metadata the vector store grants come from, taken from the auth's serde form
#[derive(Debug, Deserialize)]
pub(super) struct Grants {
    #[serde(default)]
    pub(super) metadata: Option<Payload>,
    #[serde(default)]
    pub(super) team_metadata: Option<Payload>,
}

pub(super) fn caller_grants(auth: &UserAPIKeyAuth) -> Result<Grants, VectorEndpointAccessError> {
    serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(VectorEndpointAccessError::Grants)
}

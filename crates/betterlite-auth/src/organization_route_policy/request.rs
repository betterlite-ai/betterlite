use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::organization_route_policy::error::RoutePolicyError;

// request_body.get("organization_id", None) at the declared str | None type: absent and null are the same None.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PassedOrganizationId {
    Absent,
    Text(String),
}

impl PassedOrganizationId {
    pub fn of(body: &JsonObject) -> Result<Self, RoutePolicyError> {
        match body.get("organization_id") {
            None | Some(Json::Null) => Ok(Self::Absent),
            Some(Json::Text(text)) => Ok(Self::Text(text.clone())),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                Err(RoutePolicyError::OrganizationIdNotText)
            }
        }
    }
}

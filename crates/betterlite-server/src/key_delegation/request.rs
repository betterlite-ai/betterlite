use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;

use crate::key_fields::base::KeyRequestBase;

// One request field the way pydantic holds it: model_fields_set says the caller sent it, apart from its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent<T> {
    pub supplied: bool,
    pub value: Option<T>,
}

// The key request fields the delegation checks read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDelegationRequest {
    pub user_id: Sent<String>,
    pub allowed_routes: Sent<Vec<JsonData>>,
    pub permissions: Sent<Payload>,
}

impl From<&KeyRequestBase> for KeyDelegationRequest {
    fn from(request: &KeyRequestBase) -> Self {
        Self {
            user_id: Sent {
                supplied: request.base.user_id.is_some(),
                value: request.base.user_id.clone().flatten(),
            },
            allowed_routes: Sent {
                supplied: request.key_fields.allowed_routes.is_some(),
                value: request.key_fields.allowed_routes.clone().flatten(),
            },
            permissions: Sent {
                supplied: request.base.permissions.is_some(),
                value: request.base.permissions.clone().flatten(),
            },
        }
    }
}

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::end_user::{
    CostTrackingService, END_USER_ID_FIELD, EndUserCostTrackingSettings, EndUserIdError,
    get_end_user_id_for_cost_tracking,
};

// LiteLLM lazily imports get_end_user_id_for_cost_tracking to keep utils.py out of the prometheus import; in Rust the same call costs nothing
pub trait EndUserResolver: Send + Sync {
    fn resolve_end_user(&self, end_user: Option<&str>) -> Result<Option<String>, EndUserIdError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CostTrackingEndUserResolver {
    pub settings: EndUserCostTrackingSettings,
}

impl EndUserResolver for CostTrackingEndUserResolver {
    fn resolve_end_user(&self, end_user: Option<&str>) -> Result<Option<String>, EndUserIdError> {
        let mut params = JsonObject::new();
        params.insert(
            END_USER_ID_FIELD.to_owned(),
            end_user.map_or(Json::Null, |id| Json::Text(id.to_owned())),
        );
        get_end_user_id_for_cost_tracking(&params, CostTrackingService::Prometheus, &self.settings)
    }
}

#[path = "end_user_resolution/tests.rs"]
mod tests;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::skills::get_delete::call::SkillsCallParams;

// create_skill's arguments plus the caller's kwargs, which the hosted path reads for description, instructions and user_id
pub struct CreateSkillCall<'call> {
    pub params: SkillsCallParams,
    pub display_title: Option<&'call str>,
    pub files: Option<&'call [Json]>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub kwargs: &'call IndexMap<String, Json>,
    pub litellm_call_id: Option<&'call str>,
}

// one create call together with the logging state its pre-call update writes into
pub struct SkillCreation<'call> {
    pub call: &'call CreateSkillCall<'call>,
    pub logging: &'call mut LoggingState,
}

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::skills::get_delete::call::SkillsCallParams;

// list_skills' arguments plus the caller's kwargs, which the hosted path reads for the principal and the log line reads for metadata
pub struct ListSkillsCall<'call> {
    pub params: SkillsCallParams,
    pub limit: Option<i64>,
    pub page: Option<&'call str>,
    pub source: Option<&'call str>,
    pub kwargs: &'call IndexMap<String, Json>,
    pub litellm_call_id: Option<&'call str>,
}

// one list call together with the logging state its pre-call update writes into
pub struct SkillListing<'call> {
    pub call: &'call ListSkillsCall<'call>,
    pub logging: &'call mut LoggingState,
}

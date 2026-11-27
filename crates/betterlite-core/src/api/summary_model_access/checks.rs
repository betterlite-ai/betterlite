use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::api::summary_model_access::error::PolicyFailure;

// one _can_object_call_model call: compact.py passes no key aliases, so the grant never rewrites the name
pub struct ObjectCall<'call> {
    pub model: &'call str,
    pub models: &'call [String],
    pub team_model_aliases: &'call BTreeMap<String, String>,
    pub team_id: Option<&'call str>,
    pub object_type: ModelAccessObject,
}

// B747's matcher, B10051's user grant and B886's project grant behind one seam, so a test scripts each outcome
pub trait SummaryModelChecks: Sync {
    type Failure: Display;

    // _can_object_call_model at key, team and member scope
    fn object_call(
        &self,
        router: Option<&dyn ModelAccessView>,
        call: &ObjectCall<'_>,
    ) -> Result<(), PolicyFailure<Self::Failure>>;

    // can_user_call_model over the loaded user row
    fn user_call(
        &self,
        router: Option<&dyn ModelAccessView>,
        model: &str,
        user: &LiteLLMUserTable,
    ) -> Result<(), PolicyFailure<Self::Failure>>;

    // can_project_access_model over a project whose allowlist is not empty
    fn project_call(
        &self,
        router: Option<&dyn ModelAccessView>,
        model: &str,
        project: &ProjectTable,
    ) -> Result<(), PolicyFailure<Self::Failure>>;
}

mod engine;
pub mod error;
mod metadata;
mod pipelines;
mod policies;
mod requested_guardrails;

mod tests;

use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::registry::PolicyRegistry;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_guardrail_policy::error::RequestGuardrailPolicyError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardrailPrincipal {
    pub team_alias: Option<String>,
    pub key_alias: Option<String>,
}

// the only two keys this slice reads from key, team or project metadata; every other key belongs to another slice
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct GuardrailMetadata {
    #[serde(default)]
    pub guardrails: Vec<String>,
    #[serde(default)]
    pub policies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestGuardrailPolicyInput {
    pub data: RequestData,
    pub principal: GuardrailPrincipal,
    pub key_metadata: GuardrailMetadata,
    pub team_metadata: GuardrailMetadata,
    pub project_metadata: GuardrailMetadata,
    pub metadata_variable_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardrailPolicySettings {
    pub premium_user: bool,
    pub limits: ApiLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestGuardrailPolicyResult {
    pub data: RequestData,
    pub excluded_policies: Vec<String>,
}

pub struct GuardrailPolicyService<'repositories> {
    pub policies: &'repositories PolicyRegistry,
    pub attachments: &'repositories AttachmentRegistry,
    pub settings: GuardrailPolicySettings,
}

impl GuardrailPolicyService<'_> {
    pub fn resolve(
        &self,
        input: &RequestGuardrailPolicyInput,
    ) -> Result<RequestGuardrailPolicyResult, RequestGuardrailPolicyError> {
        let mut data = input.data.clone();
        let metadata_name = input.metadata_variable_name.as_str();
        metadata::move_include_flag(&mut data, metadata_name)?;
        metadata::add_direct_guardrails(input, &mut data, metadata_name, &self.settings)?;
        let excluded_policies = policies::add_policy_guardrails(
            input,
            self.policies,
            &mut data,
            metadata_name,
            &self.settings,
        )?;
        engine::add_policy_engine_guardrails(self, input, &mut data)?;
        metadata::move_request_guardrails(&mut data, metadata_name)?;
        Ok(RequestGuardrailPolicyResult {
            data,
            excluded_policies,
        })
    }
}

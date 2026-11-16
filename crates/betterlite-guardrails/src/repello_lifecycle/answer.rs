use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use serde::Deserialize;

use crate::repello_content::wire::{RepelloAIAnalyzeResponse, RepelloAIViolatedPolicy};

// The analyze answer as Argus sends it: OpaqueParams reads every number in the policy details as the number it is.
#[derive(Deserialize)]
pub(super) struct Answer {
    verdict: Option<String>,
    request_id: Option<String>,
    policies_violated: Option<Vec<ViolatedPolicy>>,
    policies_applied: Option<Vec<OpaqueParams>>,
}

#[derive(Deserialize)]
struct ViolatedPolicy {
    policy_name: Option<String>,
    policy_id: Option<String>,
    action_taken: Option<String>,
    scope: Option<String>,
    details: Option<OpaqueParams>,
    masked_result: Option<String>,
}

impl From<Answer> for RepelloAIAnalyzeResponse {
    fn from(answer: Answer) -> Self {
        Self {
            verdict: answer.verdict,
            request_id: answer.request_id,
            policies_violated: answer
                .policies_violated
                .map(|policies| policies.into_iter().map(Into::into).collect()),
            policies_applied: answer
                .policies_applied
                .map(|applied| applied.into_iter().map(|policy| policy.0).collect()),
        }
    }
}

impl From<ViolatedPolicy> for RepelloAIViolatedPolicy {
    fn from(policy: ViolatedPolicy) -> Self {
        Self {
            policy_name: policy.policy_name,
            policy_id: policy.policy_id,
            action_taken: policy.action_taken,
            scope: policy.scope,
            details: policy.details.map(|details| details.0),
            masked_result: policy.masked_result,
        }
    }
}

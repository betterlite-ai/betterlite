use betterlite_models::decisions::Json;
use betterlite_policy::pipeline::GuardrailPipeline;
use betterlite_policy::resolver::pipelines::pipeline_managed_guardrails;
use indexmap::IndexMap;

use crate::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use crate::api::request_guardrail_policy::metadata::json_from_serialize;

const GUARDRAIL_PIPELINES: &str = "_guardrail_pipelines";
const MANAGED_GUARDRAILS: &str = "_pipeline_managed_guardrails";

pub(super) fn write_pipelines(
    bucket: &mut IndexMap<String, Json>,
    pipelines: &[(String, GuardrailPipeline)],
) -> Result<(), RequestGuardrailPolicyError> {
    if pipelines.is_empty() {
        return Ok(());
    }
    let managed = pipeline_managed_guardrails(pipelines);
    tracing::debug!(
        "Policy engine: resolved {} pipeline(s), managed guardrails: {:?}",
        pipelines.len(),
        managed
    );
    bucket.insert(
        GUARDRAIL_PIPELINES.to_owned(),
        json_from_serialize(pipelines)?,
    );
    bucket.insert(
        MANAGED_GUARDRAILS.to_owned(),
        Json::List(managed.into_iter().map(Json::Text).collect()),
    );
    Ok(())
}

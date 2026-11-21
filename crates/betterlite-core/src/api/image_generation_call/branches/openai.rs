use betterlite_logging::logging_state::LoggingState;

use super::super::call::ImageGenerationCall;
use super::super::error::{ImageGenerationException, raise, value_failure};
use super::super::operations::{
    BedrockRequest, BlackForestLabsRequest, Branch, BranchAnswer, Headers,
    ImageGenerationOperations, OpenAiRequest, Params,
};
use super::super::plan::{ImageGenerationPlan, MappedOptions, PreparedCall};
use super::header_object;

pub(super) async fn send<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<BranchAnswer, ImageGenerationException>
where
    Operations: ImageGenerationOperations + Sync,
{
    let optional_params =
        with_extra_headers(&mapped.params, prepared.generic.extra_headers.as_ref());
    let api_key = call.api_key.clone().or_else(|| prepared.api_key.clone());
    plan.operations
        .send(
            Branch::OpenAi(OpenAiRequest {
                model: prepared.model.as_deref(),
                prompt: &call.prompt,
                timeout: prepared.timeout,
                api_key: api_key.as_ref(),
                api_base: prepared.api_base.as_deref(),
                optional_params: &optional_params,
                organization: prepared.generic.organization.as_deref(),
                headers: &mapped.headers,
                asynchronous: prepared.generic.aimg_generation,
                now_seconds: call.now_seconds,
            }),
            logging,
        )
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}

pub(super) async fn black_forest_labs<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<BranchAnswer, ImageGenerationException>
where
    Operations: ImageGenerationOperations + Sync,
{
    let Some(model) = prepared.model.as_deref() else {
        let failure = value_failure(
            "Model needs to be set for black_forest_labs",
            None,
            Some("black_forest_labs"),
        );
        return Err(raise(plan.redactor, failure));
    };
    plan.operations
        .send(
            Branch::BlackForestLabs(BlackForestLabsRequest {
                model,
                prompt: &call.prompt,
                optional_params: &mapped.params,
                litellm_params: &mapped.litellm_params,
                timeout: prepared.timeout,
                extra_headers: prepared.generic.extra_headers.as_ref(),
                asynchronous: prepared.generic.aimg_generation,
            }),
            logging,
        )
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}

pub(super) async fn bedrock<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<BranchAnswer, ImageGenerationException>
where
    Operations: ImageGenerationOperations + Sync,
{
    let Some(model) = prepared.model.as_deref() else {
        let failure = value_failure("Model needs to be set for bedrock", None, Some("bedrock"));
        return Err(raise(plan.redactor, failure));
    };
    plan.operations
        .send(
            Branch::Bedrock(BedrockRequest {
                model: Some(model),
                prompt: &call.prompt,
                timeout: prepared.timeout,
                api_key: call.api_key.as_ref(),
                api_base: prepared.api_base.as_deref(),
                optional_params: &mapped.params,
                asynchronous: prepared.generic.aimg_generation,
            }),
            logging,
        )
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}

// the OpenAI handler keeps extra_headers in optional_params and its request builder overlays them
fn with_extra_headers(params: &Params, extra: Option<&Headers>) -> Params {
    let mut optional_params = params.clone();
    if let Some(extra) = extra {
        optional_params.insert(String::from("extra_headers"), header_object(extra));
    }
    optional_params
}

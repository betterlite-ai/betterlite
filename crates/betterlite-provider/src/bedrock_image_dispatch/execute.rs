use std::time::SystemTime;

use betterlite_bedrock::canvas_results;
use betterlite_bedrock::image_call::execute::{dispatch_request, transform_response};
use betterlite_bedrock::image_call::plan::{
    CredentialSource, ImageCall, LogContext, prepare_request, resolve_target,
};
use betterlite_bedrock::image_call::transport::ClientFactory;
use betterlite_bedrock::image_edits::execute::dispatch_request as dispatch_edit_request;
use betterlite_bedrock::image_edits::plan::EditCall;
use betterlite_bedrock::image_edits::prepare::{
    prepare_request as prepare_edit_request, resolve_target as resolve_edit_target,
};
use betterlite_bedrock::image_edits::response::{
    EditReply, ObservedReply, transform_response as transform_edit_response,
};
use betterlite_bedrock::image_models::response::ModelResponse;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::stability_edits;
use betterlite_models::price_lookup::PriceRecord;

use crate::operation::{OperationRequest, OperationResult};

use super::call::{BedrockImageOperation, EditOperation, GenerateOperation, Transport};
use super::error::BedrockImageError;
use super::{BedrockImageRequest, BedrockImageResult};

// The closed provider enum is the only entry: a request for another arm is refused rather than guessed
pub async fn dispatch<Source, Factory>(
    request: OperationRequest,
    operation: BedrockImageOperation<'_, '_, '_>,
    source: &Source,
    transport: Transport<'_, Factory>,
    log: &mut LogContext<'_, '_>,
) -> Result<OperationResult, BedrockImageError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
{
    let OperationRequest::BedrockImage(BedrockImageRequest) = request else {
        return Err(BedrockImageError::UnsupportedOperation);
    };
    let result = match operation {
        BedrockImageOperation::Generate(generate_operation) => BedrockImageResult::Generate(
            Box::new(generate(&generate_operation, source, transport, log).await?),
        ),
        BedrockImageOperation::Edit(edit_operation) => BedrockImageResult::Edit(Box::new(
            edit(&edit_operation, source, transport, log).await?,
        )),
        BedrockImageOperation::Unsupported { operation: name } => {
            return Err(BedrockImageError::Unsupported { operation: name });
        }
    };
    Ok(OperationResult::BedrockImage(result))
}

async fn generate<Source, Factory>(
    operation: &GenerateOperation<'_>,
    source: &Source,
    transport: Transport<'_, Factory>,
    log: &mut LogContext<'_, '_>,
) -> Result<ModelResponse, BedrockImageError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
{
    let Some(model) = operation.model else {
        return Err(BedrockImageError::ModelRequired);
    };
    // LiteLLM's generation arm forwards neither headers nor extra_headers to the Bedrock handler
    let mut call = ImageCall {
        model,
        prompt: operation.prompt,
        optional_params: operation.optional_params.clone(),
        api_base: operation.api_base,
        extra_headers: None,
        api_key: operation.api_key,
        timeout: operation.timeout,
        aimg_generation: operation.aimg_generation,
        environment: operation.environment,
        now: operation.now,
    };
    let target = resolve_target(&mut call, source).await?;
    let prepared = prepare_request(&mut call, &target, log)?;
    let reply = dispatch_request(&call, &prepared, transport.supplied, transport.factory).await?;
    let response = ModelResponse {
        created: Some(operation.created),
        data: Vec::new(),
    };
    transform_response(&call, &prepared, &reply, response, log).map_err(BedrockImageError::from)
}

async fn edit<Source, Factory>(
    operation: &EditOperation<'_, '_, '_>,
    source: &Source,
    transport: Transport<'_, Factory>,
    log: &mut LogContext<'_, '_>,
) -> Result<EditReply, BedrockImageError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
{
    let Some(model) = operation.model else {
        return Err(BedrockImageError::ModelRequired);
    };
    let now_seconds = unix_seconds(operation.now)?;
    let mut call = edit_call(operation, model);
    let target = resolve_edit_target(&mut call, source).await?;
    let prepared = prepare_edit_request(&mut call, &operation.plan, &target, log)?;
    let reply =
        dispatch_edit_request(&call, &prepared, transport.supplied, transport.factory).await?;
    let observed = ObservedReply {
        prepared: &prepared,
        reply: &reply,
        now_seconds,
    };
    edit_reply(operation, &call, &observed, log)
}

// The edit arm's call, field for field; the auth keys stay for the target resolver to pop
fn edit_call<'call>(
    operation: &'call EditOperation<'_, '_, '_>,
    model: &'call str,
) -> EditCall<'call> {
    EditCall {
        model,
        prompt: operation.prompt,
        images: operation.images,
        optional_params: operation.optional_params.clone(),
        api_base: operation.api_base,
        extra_headers: operation.extra_headers,
        api_key: operation.api_key,
        timeout: operation.timeout,
        aimage_edit: operation.aimage_edit,
        environment: operation.environment,
        now: operation.now,
    }
}

// The reply transform reads the raw catalog entry the arm already resolved for its family
fn edit_reply(
    operation: &EditOperation<'_, '_, '_>,
    call: &EditCall<'_>,
    observed: &ObservedReply<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<EditReply, BedrockImageError> {
    let prices = ModelPrices(operation.plan.model_cost);
    transform_edit_response(call, &operation.plan, observed, &prices, log)
        .map_err(BedrockImageError::from)
}

// LiteLLM's ImageResponse carries int(time.time()); the leaf passes the instant it read seconds from
fn unix_seconds(now: SystemTime) -> Result<i64, BedrockImageError> {
    let elapsed = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_error| BedrockImageError::Clock)?;
    i64::try_from(elapsed.as_secs()).map_err(|_error| BedrockImageError::Clock)
}

// The raw catalog entry, read once for whichever family the edit selector chose
struct ModelPrices<'cost>(&'cost ParamMap);

// get_model_info reads the raw entry; an absent or non-object entry has no price record
fn price_record(model_cost: &ParamMap, model: &str) -> Option<PriceRecord> {
    let entry = model_cost.get(model)?;
    let value = serde_json::to_value(entry).ok()?;
    serde_json::from_value(value).ok()
}

impl canvas_results::ModelPriceLookup for ModelPrices<'_> {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        price_record(self.0, model)
    }
}

impl stability_edits::reply::ModelPriceLookup for ModelPrices<'_> {
    fn price_record(&self, model: &str) -> Option<PriceRecord> {
        price_record(self.0, model)
    }
}

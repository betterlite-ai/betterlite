use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_models::protocol_cost::protocol_charge;
use rust_decimal::Decimal;

use super::error::SpecialChargeError;
use super::media::video_charge;
use super::request::{SpecialRequest, SpecialServices};
use super::search::search_charge;
use crate::cost::context::pricing::{PricingQuery, deployment_info};
use crate::cost::image_dispatch::policy::{ImagePricePolicy, ImageResponseView};
use crate::cost::image_dispatch::route::{ImageCall, route_image_cost};
use crate::cost::realtime::pricing::{RealtimeCostRequest, stream_cost};
use crate::routes::CallType;

// The model and provider after the cost stage stripped a provider prefix from an unresolved one.
#[derive(Clone, Copy)]
pub(super) struct Identity<'identity> {
    pub(super) model: &'identity str,
    pub(super) provider: Option<&'identity str>,
}

fn image_charge<Resolver, ImagePolicy>(
    request: &SpecialRequest<'_>,
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    identity: Identity<'_>,
    wire_call_type: &str,
) -> Result<Option<Decimal>, SpecialChargeError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
{
    let image = &request.response.image;
    // Only a parsed ImageResponse is priced here; any other response falls through to the ordinary stage.
    if matches!(image.response, ImageResponseView::Other) {
        return Ok(None);
    }
    let query = PricingQuery {
        custom_pricing: request.custom_pricing,
        router_model_id: request.router_model_id,
        logging_params: request.logging.params,
    };
    let model_info = deployment_info(&query, services.lookup)?;
    let charge = route_image_cost(
        services.image_policy,
        services.size_pattern,
        &ImageCall {
            model: identity.model,
            custom_llm_provider: identity.provider,
            quality: image.quality,
            image_count: image.image_count,
            size: image.size,
            optional_params: request.optional_params,
            call_type: Some(wire_call_type),
            model_info: model_info.as_ref(),
            response: image.response,
        },
    )?;
    Ok(Some(charge))
}

fn realtime_charge<Resolver, ImagePolicy, Target>(
    request: &SpecialRequest<'_>,
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    identity: Identity<'_>,
    target: Option<&mut Target>,
) -> Result<Option<Decimal>, SpecialChargeError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
{
    let Some(session) = &request.response.realtime else {
        return Ok(None);
    };
    let (Some(usage), Some(provider)) = (session.usage, identity.provider) else {
        return Err(SpecialChargeError::RealtimeInputs);
    };
    let custom_pricing = request.custom_pricing == Some(true);
    let charge = stream_cost(
        &RealtimeCostRequest {
            results: session.results,
            combined_usage: usage,
            custom_llm_provider: provider,
            litellm_model_name: identity.model,
            data_residency: request.data_residency,
            custom_pricing_model: request.selected_model.filter(|_| custom_pricing),
            base_pricing_model: request
                .selected_model
                .filter(|_| request.base_model.is_some() && !custom_pricing),
            moment: request.moment,
        },
        services.lookup,
        services.database,
        target,
    )?;
    Ok(Some(charge))
}

// The passthrough image call has no CallType, so the caller flags it; its wire name is spelled with hyphens.
const PASSTHROUGH_IMAGE_CALL: &str = "passthrough-image-generation";

const IMAGE_CALLS: [(CallType, &str); 4] = [
    (CallType::ImageGeneration, "image_generation"),
    (CallType::AimageGeneration, "aimage_generation"),
    (CallType::ImageEdit, "image_edit"),
    (CallType::AimageEdit, "aimage_edit"),
];

// The calls that end here with a finished charge; None means the ordinary stage continues.
pub(super) fn early_charge<Resolver, ImagePolicy, Target>(
    request: &SpecialRequest<'_>,
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    identity: Identity<'_>,
    target: Option<&mut Target>,
) -> Result<Option<Decimal>, SpecialChargeError<ImagePolicy::Error>>
where
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
{
    let call_type = request.call_type;
    let image_wire = if request.passthrough_image {
        Some(PASSTHROUGH_IMAGE_CALL)
    } else {
        IMAGE_CALLS
            .iter()
            .find(|(image, _)| *image == call_type)
            .map(|(_, wire)| *wire)
    };
    if let Some(wire) = image_wire {
        return image_charge(request, services, identity, wire);
    }
    if matches!(
        call_type,
        CallType::CreateVideo
            | CallType::AcreateVideo
            | CallType::VideoEdit
            | CallType::AvideoEdit
            | CallType::VideoRemix
            | CallType::AvideoRemix
    ) {
        return video_charge(request, services.lookup, identity.model, identity.provider).map(Some);
    }
    if matches!(call_type, CallType::Search | CallType::Asearch) {
        return search_charge(request, services, identity, target).map(Some);
    }
    if call_type == CallType::PrivateArealtime {
        return realtime_charge(request, services, identity, target);
    }
    if call_type == CallType::CallMcpTool {
        let charge = request.response.protocol.map(protocol_charge).transpose()?;
        return Ok(Some(charge.unwrap_or_default()));
    }
    Ok(None)
}

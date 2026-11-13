use std::time::SystemTime;

use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_models::protocol_cost::protocol_charge;

use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::{AttemptOutcome, PreparedBillingAttempt};
use crate::routes::CallType;
use early::{Identity, early_charge};
use error::SpecialChargeError;
use media::record_media;
use policy::ModelResolver;
use request::{SpecialOutcome, SpecialRequest, SpecialServices};
use tail::token_tail;

mod early;
pub mod error;
mod media;
pub mod policy;
pub mod replicate;
pub mod request;
mod search;
mod tail;
mod tests;

// Ports completion_cost L1562-L1810: either a finished charge or the attempt the ordinary token stage continues with.
pub fn special_charge<Resolver, ImagePolicy, Target, Clock>(
    services: &SpecialServices<'_, Resolver, ImagePolicy>,
    request: &SpecialRequest<'_>,
    mut attempt: PreparedBillingAttempt,
    clock: &mut Clock,
    target: Option<&mut Target>,
) -> Result<SpecialOutcome, SpecialChargeError<ImagePolicy::Error>>
where
    Resolver: ModelResolver,
    ImagePolicy: ImagePricePolicy,
    Target: CostBreakdownTarget,
    Clock: FnMut() -> SystemTime,
{
    if !matches!(attempt.outcome, AttemptOutcome::Ready(_)) {
        return Ok(SpecialOutcome::Failed(attempt));
    }
    // An A2A call needs no model, so it ends before the model check.
    if matches!(
        request.call_type,
        CallType::SendMessage | CallType::AsendMessage
    ) {
        let charge = request.response.protocol.map(protocol_charge).transpose()?;
        return Ok(SpecialOutcome::Charged(charge.unwrap_or_default()));
    }
    let mut model = attempt
        .model
        .clone()
        .ok_or(SpecialChargeError::ModelMissing)?;
    if attempt.custom_llm_provider.is_none() {
        match services.resolver.resolve_model(&model) {
            Ok(resolved) => {
                model = resolved.model;
                attempt.custom_llm_provider = Some(resolved.provider.slug().to_owned());
            }
            Err(failure) => {
                tracing::debug!(?failure, "Error inferring custom_llm_provider");
            }
        }
        attempt.model = Some(model.clone());
    }
    let identity = Identity {
        model: &model,
        provider: attempt.custom_llm_provider.as_deref(),
    };
    if let Some(charge) = early_charge(request, services, identity, target)? {
        return Ok(SpecialOutcome::Charged(charge));
    }
    if let AttemptOutcome::Ready(billing) = &mut attempt.outcome {
        record_media(request, billing);
    }
    token_tail(request, services, attempt, clock)
}

use betterlite_core::api::chat_response_stream_tools::fields::string_value;
use betterlite_core::api::responses_tool_namespaces::values::has_value;
use betterlite_core::shadow_evidence::chat_text;
use betterlite_core::shadow_evidence::cost::call_cost;
use betterlite_core::shadow_evidence::failure::detail;
use betterlite_core::shadow_evidence::records::{CallFailure, ShadowResponse};
use betterlite_core::shadow_evidence::routing;
use betterlite_logging::logging_privacy::internal::{self, Forwarding, Origin};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{
    NO_ROUTER_ERROR, ShadowCall, ShadowDeps, ShadowRequest, ShadowRouterError, metadata_failure,
};

mod tests;

pub async fn call_router_shadow(
    request: &ShadowRequest<'_>,
    deps: &ShadowDeps<'_>,
) -> Result<ShadowResponse, CallFailure> {
    let Some(router) = deps.provider.shadow_router() else {
        return Err(no_router_failure());
    };
    let mut metadata = internal::forward(
        request.parent_metadata,
        Origin::ShadowEvalRouter,
        Forwarding::Detached,
    )
    .map_err(metadata_failure)?;
    let call = ShadowCall {
        model: request.target_model,
        messages: request.messages.to_vec(),
        metadata: &mut metadata,
        num_retries: 0,
        fallbacks: Vec::new(),
        params: request.shadow_params.clone(),
    };
    let outcome = router.acompletion(call).await;
    match outcome {
        Ok(response) => shadow_response(&response, metadata, deps),
        Err(error) => Err(router_failure(&error, &metadata)),
    }
}

fn no_router_failure() -> CallFailure {
    CallFailure {
        error: NO_ROUTER_ERROR.to_owned(),
        cost: Decimal::ZERO,
        classifier_cost: Decimal::ZERO,
    }
}

fn router_failure(error: &ShadowRouterError, metadata: &IndexMap<String, Json>) -> CallFailure {
    let failure = detail(&error.class, error.location, &error.message);
    CallFailure {
        error: format!("shadow router call failed: {failure}"),
        cost: Decimal::ZERO,
        classifier_cost: routing::classifier_cost(&Json::Object(metadata.clone())),
    }
}

fn shadow_response(
    response: &Json,
    metadata: IndexMap<String, Json>,
    deps: &ShadowDeps<'_>,
) -> Result<ShadowResponse, CallFailure> {
    let text = chat_text::final_text(response);
    let metadata = Json::Object(metadata);
    let decision = routing::decision(&metadata);
    let model = served_model(response, &decision);
    let classifier_cost = routing::classifier_cost(&metadata);
    let cost = call_cost(response, deps.prices);
    if text.is_empty() {
        return Err(CallFailure {
            error: chat_text::empty_reply_error(response, &model),
            cost,
            classifier_cost,
        });
    }
    Ok(ShadowResponse {
        text,
        model,
        tier: routing::routed_tier(&metadata),
        cost,
        classifier_cost,
    })
}

fn served_model(response: &Json, decision: &IndexMap<String, Json>) -> String {
    let served = chat_text::field(response, "model").filter(|value| has_value(value));
    let routed = decision
        .get("routed_model")
        .filter(|value| has_value(value));
    served.or(routed).map_or_else(String::new, string_value)
}

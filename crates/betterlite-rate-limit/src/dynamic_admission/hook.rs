use std::ops::Mul as _;

use betterlite_core::api::request_destination::RequestData;
use betterlite_core::routes::CallType;
use betterlite_error::rate_limit::ProxyRateLimitError;
use betterlite_models::decisions::Json;

use crate::dynamic_priority::resolution::priority_from_key_dict;
use crate::dynamic_priority::saturation::check_model_saturation;
use crate::fail_closed::RateLimitUnverifiableError;
use crate::provider_attribution::ModelGroupLimits;
use crate::request_stash::RequestStashSlot;

use super::admission::{AdmissionKey, CheckRateLimitError, DynamicAdmission, RateLimitCheck};
use super::limiter_port::AdmissionLimiterError;

mod tests;

// Why the hook cannot read a model group's capacities: no router is installed, or the router's own read failed
#[derive(Debug, thiserror::Error)]
pub enum ModelGroupInfoError {
    #[error("the dynamic rate limiter has no router to read the model group info of {model} from")]
    NoRouter { model: String },
    #[error("the router could not read the model group info of {model}")]
    Unavailable { model: String },
}

// Router.get_model_group_info: a group's tpm and rpm, None for a group with no deployments, and a failure the hook never catches
pub trait ModelGroupInfoSource: Send + Sync {
    fn model_group_info(
        &self,
        model: &str,
    ) -> Result<Option<ModelGroupLimits>, ModelGroupInfoError>;
}

// async_pre_call_hook's ways out: allowed, the 429, the fail-closed 503, or the router failure that stays uncaught
#[derive(Debug, thiserror::Error)]
pub enum PreCallError {
    #[error(transparent)]
    ModelGroupInfo(#[from] ModelGroupInfoError),
    #[error(transparent)]
    Rejection(Box<ProxyRateLimitError>),
    #[error(transparent)]
    Unverifiable(#[from] RateLimitUnverifiableError),
}

impl<Router: ModelGroupInfoSource> DynamicAdmission<Router> {
    // async_pre_call_hook: claim the stash, read the group and its saturation, then admit or reject without ever changing the body
    pub async fn pre_call(
        &self,
        key: &AdmissionKey<'_>,
        data: &RequestData,
        _call_type: CallType,
        stash: &RequestStashSlot,
    ) -> Result<(), PreCallError> {
        let Some(Json::Text(raw_model)) = data.get("model") else {
            return Ok(());
        };
        let model = raw_model.as_str();
        let call_id = match data.get("litellm_call_id") {
            Some(Json::Text(text)) => Some(text.as_str()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        };
        drop(stash.claim_for_call_id(call_id));
        let priority = priority_from_key_dict(key.team_metadata, key.key_metadata);
        let Some(model_group_info) = self.router_group_info(model)? else {
            tracing::debug!("No model group info for {model}, allowing request");
            return Ok(());
        };
        let saturation = check_model_saturation(
            model,
            &model_group_info,
            self.handler.internal_usage_cache.as_ref(),
            &self.handler.policy,
        )
        .await;
        let threshold = self.handler.policy.settings.saturation_threshold;
        tracing::debug!(
            "[Dynamic Rate Limiter] Model={model}, Saturation={:.1}%, Threshold={:.1}%, Priority={}",
            saturation.mul(100.0_f64),
            threshold.mul(100.0_f64),
            priority.unwrap_or("None"),
        );
        let check = RateLimitCheck {
            model,
            model_group_info,
            key,
            priority,
            saturation,
            stash,
        };
        handle_check_result(self.check_rate_limits(&check).await)
    }

    // LiteLLM asks the router before its try block, so a failing group read reaches the caller instead of failing open
    fn router_group_info(&self, model: &str) -> Result<Option<ModelGroupLimits>, PreCallError> {
        let Some(router) = self.handler.llm_router.as_deref() else {
            return Err(PreCallError::ModelGroupInfo(
                ModelGroupInfoError::NoRouter {
                    model: model.to_owned(),
                },
            ));
        };
        router
            .model_group_info(model)
            .map_err(PreCallError::ModelGroupInfo)
    }
}

// LiteLLM re-raises HTTPExceptions (the 429 and the fail-closed 503) and logs and allows every other failure
fn handle_check_result(result: Result<(), CheckRateLimitError>) -> Result<(), PreCallError> {
    match result {
        Ok(()) => Ok(()),
        Err(CheckRateLimitError::Rejection(error)) => Err(PreCallError::Rejection(error)),
        Err(CheckRateLimitError::Limiter(AdmissionLimiterError::Unverifiable(error))) => {
            Err(PreCallError::Unverifiable(error))
        }
        Err(other) => {
            tracing::error!("Error in dynamic rate limiter: {other}, allowing request");
            Ok(())
        }
    }
}

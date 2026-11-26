use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::proxy_error_types::ModelAccessObject;

use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::TeamLookup;
use crate::entity_model_access::error::EntityModelAccessError;
use crate::entity_model_access::grant::{entity_grant, key_grant, text_models};
use crate::entity_model_access::key::can_key_call_model;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::limits::KeyModelBudgetLimiter;
use super::request_views::{ModelRewrite, RewriteFailure, rewrite_model};

// _check_key_model_budget_with_fallback: only a budget error reaches the fallback path, and only the top-level model can move
#[derive(Debug, thiserror::Error)]
pub enum KeyBudgetError {
    #[error("the key's per-model budget was exceeded")]
    Budget(Box<BudgetExceededDetails>),
    #[error("the per-model budget limiter failed")]
    Limiter(Box<ErrorClass>),
    #[error(transparent)]
    Access(EntityModelAccessError),
    #[error(transparent)]
    Rewrite(RewriteFailure),
}

#[derive(Clone, Copy)]
pub struct KeyBudgetCall<'call> {
    pub limiter: &'call dyn KeyModelBudgetLimiter,
    pub token: &'call UserAPIKeyAuth,
    pub model_name: &'call str,
    pub request_model: Option<&'call str>,
}

pub async fn check_key_budget_fallback(
    call: KeyBudgetCall<'_>,
    request: &mut dyn ModelRewrite,
    context: &ModelAccessContext<'_>,
    lookup: Option<&TeamLookup>,
) -> Result<(), KeyBudgetError> {
    let KeyBudgetCall {
        limiter,
        token,
        model_name,
        request_model,
    } = call;
    let exceeded = match limiter.is_key_within_model_budget(token, model_name).await {
        Ok(_within) => return Ok(()),
        Err(ErrorClass::BudgetExceeded(details)) => details,
        Err(other) => return Err(KeyBudgetError::Limiter(Box::new(other))),
    };
    if request_model != Some(model_name) {
        return Err(KeyBudgetError::Budget(exceeded));
    }
    let selected = limiter
        .fallback_model_within_budget(token, model_name)
        .await
        .map_err(|error| KeyBudgetError::Limiter(Box::new(error)))?;
    let Some(fallback) = selected else {
        return Err(KeyBudgetError::Budget(exceeded));
    };
    if let Err(error) = authorize_fallback(&fallback, token, context, lookup).await {
        return if error.denial().is_some() {
            Err(KeyBudgetError::Budget(exceeded))
        } else {
            Err(KeyBudgetError::Access(error))
        };
    }
    rewrite_model(request, &fallback).map_err(KeyBudgetError::Rewrite)?;
    Ok(())
}

// can_key_call_model first, then only a nonempty team list needs B747's team grant with the token's aliases
async fn authorize_fallback(
    fallback: &str,
    token: &UserAPIKeyAuth,
    context: &ModelAccessContext<'_>,
    lookup: Option<&TeamLookup>,
) -> Result<(), EntityModelAccessError> {
    let requested = RequestedModels::One(fallback.to_owned());
    can_key_call_model(&requested, token, context, lookup).await?;
    let team_models = text_models(&token.view.team_models);
    if team_models.is_empty() {
        return Ok(());
    }
    let key = key_grant(token);
    let grant = entity_grant(
        team_models,
        Some(key.team_model_aliases),
        Some(key.key_model_aliases),
        key.team_id,
    );
    EntityModelAccessError::gate(can_object_call_model(
        &requested,
        &grant,
        ModelAccessObject::Team,
        context,
        0,
    ))
}

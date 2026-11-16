use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use betterlite_auth::auth_model_patterns::denial::ModelAccessDecision;
use betterlite_auth::auth_model_patterns::grant::ModelGrantPattern;
use betterlite_auth::auth_model_patterns::pattern::{
    ProviderInference, UnresolvedProviderInference,
};
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_config::sections::database_reencryption::DEFAULT_RECURSE_DEPTH;
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_error::proxy_error_types::ModelAccessObject;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;

use crate::auth::auth_identity::AuthContext;
use crate::state::ServerState;

mod tests;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

// the authenticated caller's model grant, supplied by the stage that resolved the key or team
#[derive(Debug)]
pub struct ModelPatternRule {
    pub object: ModelAccessObject,
    pub grant: ModelGrantPattern,
    pub view: Option<Arc<dyn ModelAccessView>>,
    pub provider: Option<Arc<dyn ProviderInference>>,
    pub global_model_aliases: BTreeMap<String, String>,
}

impl Default for ModelPatternRule {
    fn default() -> Self {
        Self {
            object: ModelAccessObject::Key,
            grant: ModelGrantPattern::default(),
            view: None,
            provider: None,
            global_model_aliases: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModelPatternStageInput {
    pub context: AuthContext,
    pub requested: RequestedModels,
    pub rule: Arc<ModelPatternRule>,
    pub max_recurse_depth: i64,
}

#[derive(Debug)]
pub enum ModelAccessRejection {
    // ast-grep-ignore: no-long-names
    Denied(Box<ModelAccessDeniedProxyException>),
    RecurseDepth { model: String },
}

pub(crate) fn run_stage(
    stage: &ModelPatternStageInput,
) -> Result<AuthContext, ModelAccessRejection> {
    let provider: &dyn ProviderInference = stage
        .rule
        .provider
        .as_deref()
        .unwrap_or(&UNRESOLVED_PROVIDER);
    let context = ModelAccessContext {
        view: stage.rule.view.as_deref(),
        global_model_aliases: &stage.rule.global_model_aliases,
        compaction: None,
        provider,
        max_recurse_depth: stage.max_recurse_depth,
    };
    match can_object_call_model(
        &stage.requested,
        &stage.rule.grant,
        stage.rule.object,
        &context,
        0,
    ) {
        ModelAccessDecision::Allowed => Ok(stage.context.clone()),
        ModelAccessDecision::Denied(denial) => Err(ModelAccessRejection::Denied(denial)),
        ModelAccessDecision::RecurseDepthExceeded { model } => {
            Err(ModelAccessRejection::RecurseDepth { model })
        }
    }
}

pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<ModelPatternRule> {
    request
        .extensions()
        .get::<Arc<ModelPatternRule>>()
        .map_or_else(|| Arc::new(ModelPatternRule::default()), Arc::clone)
}

pub(crate) fn max_recurse_depth_for_request(request: &Request<Body>) -> i64 {
    request
        .extensions()
        .get::<Arc<ServerState>>()
        .map_or(DEFAULT_RECURSE_DEPTH, |state| {
            state.auth_model_patterns.max_recurse_depth
        })
}

// ProxyException's response fields: the client message, the object's error type, param and code
pub(crate) fn rejection_response(rejection: &ModelAccessRejection) -> Response {
    match rejection {
        ModelAccessRejection::Denied(denial) => denied_response(denial),
        ModelAccessRejection::RecurseDepth { model } => recurse_depth_response(model),
    }
}

// ast-grep-ignore: no-long-names
fn denied_response(denial: &ModelAccessDeniedProxyException) -> Response {
    let exception = denial.as_proxy_exception();
    tracing::debug!(
        error_type = %exception.error_type,
        internal_message = %denial.sanitized_internal_message(),
        "model access denied",
    );
    let body = ErrorResponse {
        error: ErrorObject {
            message: exception.message.clone(),
            error_type: WireField::Text(exception.error_type.clone()),
            param: exception
                .param
                .clone()
                .map_or(WireField::Null, WireField::Text),
            code: WireField::Text(exception.code.clone()),
        },
    };
    (StatusCode::FORBIDDEN, Json(body)).into_response()
}

// a bare Exception becomes auth_error with param "None" and status 401
fn recurse_depth_response(model: &str) -> Response {
    let message = format!(
        "Authentication Error, Unable to parse model, max fallback depth exceeded - \
         received model: {model}"
    );
    let body = ErrorResponse {
        error: ErrorObject {
            message,
            error_type: WireField::Text("auth_error".to_owned()),
            param: WireField::MISSING,
            code: WireField::Text("401".to_owned()),
        },
    };
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

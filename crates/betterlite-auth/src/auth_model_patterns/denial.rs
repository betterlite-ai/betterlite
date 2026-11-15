use betterlite_error::proxy_error_types::{ModelAccessObject, ProxyErrorTypes};
use betterlite_error::proxy_exception::ExceptionCode;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;

// model_access_denied.py's MODEL_ACCESS_DENIED_CLIENT_MESSAGE, formatted with the requested model
pub const MODEL_ACCESS_DENIED_MESSAGE: &str = concat!(
    "The requested model '{model}' is not available for this API key, or the model name is invalid. ",
    "Check the models available to you and try again."
);

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn model_denied_client_message(model: &str) -> String {
    MODEL_ACCESS_DENIED_MESSAGE.replace("{model}", model)
}

// the object_type literals ProxyErrorTypes.get_model_access_error_type_for_object dispatches on
#[must_use]
pub const fn model_access_object_name(object: ModelAccessObject) -> &'static str {
    match object {
        ModelAccessObject::Key => "key",
        ModelAccessObject::Team => "team",
        ModelAccessObject::User => "user",
        ModelAccessObject::Org => "org",
        ModelAccessObject::Project => "project",
        ModelAccessObject::Agent => "agent",
    }
}

// the internal message names the allowlist for the failure log, never the client response
#[must_use]
pub fn model_access_denied(
    object: ModelAccessObject,
    models: &[String],
    model: &str,
    // ast-grep-ignore: no-long-names
) -> ModelAccessDeniedProxyException {
    let object_name = model_access_object_name(object);
    // ast-grep-ignore: no-long-names
    ModelAccessDeniedProxyException::new(
        model_denied_client_message(model),
        format!(
            "{object_name} not allowed to access model. This {object_name} can only access \
             models={models:?}. Tried to access {model}"
        ),
        ProxyErrorTypes::model_access_denied_for(object).to_string(),
        Some("model".to_owned()),
        ExceptionCode::Status(403),
    )
}

// ModelAccessDeniedHTTPException is a FastAPI HTTPException carrying a diagnostic the client never sees, raised by callers that reject before the object check runs (the JWT role and scope checks)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAccessDeniedHttpException {
    internal_message: String,
    status_code: u16,
    detail: String,
}

impl ModelAccessDeniedHttpException {
    #[must_use]
    pub fn new(internal_message: String, status_code: u16, detail: String) -> Self {
        Self {
            internal_message,
            status_code,
            detail,
        }
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn status_code(&self) -> u16 {
        self.status_code
    }

    // auth_exception_handler._as_proxy_exception: an HTTPException becomes auth_error, param "None", code its own status, and its detail is the client message
    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn as_proxy_exception(&self) -> ModelAccessDeniedProxyException {
        // ast-grep-ignore: no-long-names
        ModelAccessDeniedProxyException::new(
            self.detail.clone(),
            self.internal_message.clone(),
            ProxyErrorTypes::AuthError.to_string(),
            Some("None".to_owned()),
            ExceptionCode::Status(i64::from(self.status_code)),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelAccessDecision {
    Allowed,
    // ast-grep-ignore: no-long-names
    Denied(Box<ModelAccessDeniedProxyException>),
    RecurseDepthExceeded { model: String },
}

impl ModelAccessDecision {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

use std::collections::{BTreeMap, BTreeSet};

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::api_base_lookup::{ApiBaseError, ApiBaseHooks, LoginFreeProvider};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::HelperError;

pub(crate) const MOMENT: &str = "2026-01-01T00:00:00+00:00";

pub(crate) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ResponsesStreamPrimitivesError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// B7320's callers supply these two hooks; the case gives it no login-free provider or vertex model
pub(crate) struct NoHooks;

impl ApiBaseHooks for NoHooks {
    fn login_free_api_base(&self, _provider: LoginFreeProvider) -> Option<String> {
        None
    }

    fn vertex_claude_url(
        &self,
        _location: &str,
        _project: &str,
        _model: &str,
        _stream: bool,
    ) -> Result<String, ApiBaseError> {
        Err(ApiBaseError::InvalidVertexLocation {
            message: String::new(),
        })
    }
}

pub(crate) struct SharedResolution {
    pub(crate) catalog: ModelCatalog,
    pub(crate) known: BTreeSet<String>,
    pub(crate) custom: BTreeSet<String>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) aliases: BTreeMap<String, String>,
}

impl Default for SharedResolution {
    fn default() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            aliases: BTreeMap::new(),
        }
    }
}

pub(crate) fn resolution_context(resolution: &SharedResolution) -> ResolutionContext<'_> {
    ResolutionContext {
        catalog: &resolution.catalog,
        known_model_names: &resolution.known,
        custom_providers: &resolution.custom,
        environment: &resolution.environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

// the mapped exception's .type/.param are None on the wire, so a missing field reads the same way
pub(crate) const fn wire_text(field: &WireField) -> Option<&str> {
    match field {
        WireField::Text(text) => Some(text.as_str()),
        WireField::Missing(_) | WireField::Null => None,
    }
}

// type(mapped_exception).__name__ for every class exception_type can hand back
pub(crate) const fn mapped_class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::RateLimit(_) => "RateLimitError",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::ContextWindowExceeded(_) => "ContextWindowExceededError",
        ErrorClass::ContentPolicyViolation(_) => "ContentPolicyViolationError",
        ErrorClass::ServiceUnavailable(_) => "ServiceUnavailableError",
        ErrorClass::BadGateway(_) => "BadGatewayError",
        ErrorClass::InternalServer(_) => "InternalServerError",
        ErrorClass::Api(_) => "APIError",
        ErrorClass::ApiConnection(_) => "APIConnectionError",
        ErrorClass::ApiResponseValidation(_) => "APIResponseValidationError",
        ErrorClass::JsonSchemaValidation(_) => "JSONSchemaValidationError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::GuardrailRaised(_) => "GuardrailRaisedException",
        ErrorClass::MidStreamFallback(_) => "MidStreamFallbackError",
        ErrorClass::Unclassified(_) => "Exception",
    }
}

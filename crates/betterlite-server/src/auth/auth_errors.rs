use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_errors::conversion::AuthFailure;
use betterlite_auth::auth_errors::handler::{
    AuthErrorHandler, AuthErrorSettings, FailedRequest, HandlerError,
};
use betterlite_auth::auth_errors::ports::{
    AuthErrorPorts, AuthLog, FailureHookCall, HookError, Level, Logger, NoNativeProvider,
    ServiceFailure,
};
use betterlite_auth::auth_request_policy::ip::RequestProvenance;
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use secrecy::SecretString;

use super::auth_identity::StageFailure;
use super::auth_resource_grants::denial_response;
use super::extract_key;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;
use crate::state::auth_errors::AuthFailureHook;

mod tests;

struct ServerPorts {
    hook: Option<Arc<dyn AuthFailureHook>>,
}

#[async_trait]
impl AuthErrorPorts for ServerPorts {
    // the gateway turns the outage bypass off, so nothing records a database failure here
    #[cfg_attr(test, mutants::skip)]
    fn record_service_failure(&self, failure: &ServiceFailure<'_>) {
        tracing::warn!(call_type = failure.call_type, error = %failure.error, "database service failure");
    }

    fn log(&self, entry: AuthLog) {
        let ip = entry.requester_ip.unwrap_or_default();
        let requester_ip = ip.as_str();
        let traceback = entry.traceback;
        let message = entry.message;
        match (entry.logger, entry.level) {
            (Logger::Proxy, Level::Error) => {
                tracing::error!(target: "LiteLLM Proxy", requester_ip, traceback, "{message}");
            }
            (Logger::Proxy, Level::Warning) => {
                tracing::warn!(target: "LiteLLM Proxy", requester_ip, traceback, "{message}");
            }
            (Logger::ProxyStdout, Level::Error) => {
                tracing::error!(target: "LiteLLM Proxy.stdout", requester_ip, traceback, "{message}");
            }
            (Logger::ProxyStdout, Level::Warning) => {
                tracing::warn!(target: "LiteLLM Proxy.stdout", requester_ip, traceback, "{message}");
            }
        }
    }

    // the gateway opens no OpenTelemetry server span yet, so there is nothing to stamp
    fn seed_identity(&self, _identity: &UserAPIKeyAuth, _model: Option<&Json>) {}

    fn native_resolution(&self) -> &dyn ProviderResolution {
        &NoNativeProvider
    }

    fn router(&self) -> Option<&dyn DeploymentSnapshot> {
        None
    }

    async fn post_call_failure_hook(
        &self,
        call: FailureHookCall<'_>,
    ) -> Result<Option<AuthFailure>, HookError> {
        match &self.hook {
            Some(hook) => hook.transform(call).await,
            None => Ok(None),
        }
    }
}

// what the rejection needs from the request, taken before any await since the body is not shareable
pub(crate) struct FailureContext {
    state: Option<Arc<ServerState>>,
    headers: HeaderMap,
    route: String,
    api_key: SecretString,
    // a body that is not a JSON object reaches the callbacks as an empty request
    request_data: JsonObject,
}

impl FailureContext {
    pub(crate) fn of(request: &Request<Body>, body: Option<&Bytes>) -> Self {
        Self {
            state: request.extensions().get::<Arc<ServerState>>().cloned(),
            headers: request.headers().clone(),
            route: request.uri().path().to_owned(),
            api_key: extract_key(request.headers())
                .unwrap_or_else(|| SecretString::from(String::new())),
            request_data: body
                .and_then(|bytes| serde_json::from_slice::<JsonObject>(bytes).ok())
                .unwrap_or_default(),
        }
    }

    // the handler logs every rejection and lets the failure hook replace it; a missing key keeps its own answer
    pub(crate) async fn respond(
        &self,
        failure: &StageFailure,
        failure_response: fn(&StageFailure) -> Response,
    ) -> Response {
        let Some(state) = &self.state else {
            return failure_response(failure);
        };
        let slot = &state.auth_errors;
        let (Some(routes), Some(redactor), Some(exception)) = (
            slot.routes.as_ref(),
            slot.redactor.as_ref(),
            raised_exception(failure),
        ) else {
            return failure_response(failure);
        };
        let handler = AuthErrorHandler {
            ports: &ServerPorts { hook: slot.hook() },
            // the gateway reads no key from a database, so a rejected key never earns the outage identity
            settings: AuthErrorSettings {
                allow_on_outage: false,
                ..slot.settings
            },
            routes,
            redactor,
        };
        let original = AuthFailure::Proxy {
            exception,
            invalid_virtual_key: false,
        };
        match handler.handle(original, self.request()).await {
            Err(HandlerError::Rejected(raised)) => denial_response(raised.failure.exception()),
            Ok(_) => failure_response(failure),
            // a raising callback escapes user_api_key_auth as an unhandled exception
            Err(HandlerError::Hook(_) | HandlerError::Identity(_)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
            }
        }
    }

    fn request(&self) -> FailedRequest<'_> {
        FailedRequest {
            request_data: &self.request_data,
            route: &self.route,
            parent_span: None,
            api_key: &self.api_key,
            resolved_identity: None,
            provenance: RequestProvenance {
                forwarded_for: self
                    .headers
                    .get("x-forwarded-for")
                    .and_then(|value| value.to_str().ok()),
                client_host: None,
            },
            scope_headers: Some(&self.headers),
        }
    }
}

fn raised_exception(failure: &StageFailure) -> Option<ProxyException> {
    match failure {
        StageFailure::ModelGrants(exception)
        | StageFailure::EntityModelAccess(exception)
        | StageFailure::AuthRequestPolicy(exception)
        | StageFailure::ResourceGrants(exception)
        | StageFailure::TagBudgets(exception)
        | StageFailure::RequestBodySafety(exception)
        | StageFailure::VirtualKeyRoutes(exception)
        | StageFailure::KeyBudgets(exception)
        | StageFailure::TeamBudgets(exception)
        | StageFailure::OrganizationRoutePolicy(exception)
        | StageFailure::OrgLookup(exception)
        | StageFailure::EndUsers(exception)
        | StageFailure::AuthPolicySupport(exception)
        | StageFailure::ModelGrants(exception) => Some((**exception).clone()),
        StageFailure::Authentication(authentication) => match **authentication {
            AuthenticationFailure::Invalid => Some(ProxyException::new(ProxyExceptionInit {
                message: "the API key is not valid".to_owned(),
                error_type: "no_db_connection".to_owned(),
                param: None,
                code: ExceptionCode::Status(400_i64),
                headers: BTreeMap::new(),
                openai_code: None,
                provider_specific_fields: BTreeMap::new(),
            })),
            AuthenticationFailure::Missing => None,
        },
        StageFailure::ModelAccess(_) => None,
    }
}

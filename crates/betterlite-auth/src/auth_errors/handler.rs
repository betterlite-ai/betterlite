use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_db::database_faults::is_database_connection_error;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::call_helpers::expected_error::is_expected_client_error;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::secret::Redactor;
use http::HeaderMap;
use http::header::USER_AGENT;
use secrecy::{ExposeSecret as _, SecretString};

use super::client_context::{identity_log_suffix, with_client_context};
use super::conversion::{AuthFailure, ClientFailure, as_proxy_exception};
use super::ports::{
    AuthErrorPorts, AuthLog, FailureHookCall, HookError, Level, Logger, ServiceFailure,
};
use crate::auth_request_policy::invalid_key::{
    MarkOutcome, is_invalid_virtual_key_error, mark_invalid_virtual_key_error,
};
use crate::auth_request_policy::ip::{RequestProvenance, request_ip_address};
use crate::auth_route_patterns::normalize::RouteNormalizer;
use crate::provider_resolution::resolve_rate_limit_provider;
use crate::resolved_key::auth::{
    ApiKeySecret, AuthError as IdentityError, AuthInput, ParentOtelSpan, UserAPIKeyAuth,
};
use crate::resolved_key::credential::safe_hash_api_key;
use crate::user_roles::UserRole;

pub const DB_UNAVAILABLE_FALLBACK_USER_ID: &str = "__db_unavailable_fallback__";
const OUTAGE_KEY_NAME: &str = "failed-to-connect-to-db";
const OUTAGE_WARNING: &str = "Auth: DB unavailable \u{2014} issuing restricted INTERNAL_USER fallback token (allow_requests_on_db_unavailable=True)";
const FAILURE_PREFIX: &str = "litellm.proxy.proxy_server.user_api_key_auth(): Exception occured - ";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogSwitches {
    pub client_error_tracebacks: bool,
    pub key_identity: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthErrorSettings {
    pub log: LogSwitches,
    pub allow_requests_on_db_unavailable: bool,
    pub use_x_forwarded_for: bool,
}

pub struct FailedRequest<'request> {
    pub request_data: &'request JsonObject,
    pub route: &'request str,
    pub parent_span: Option<ParentOtelSpan>,
    pub api_key: &'request SecretString,
    pub resolved_identity: Option<&'request UserAPIKeyAuth>,
    pub provenance: RequestProvenance<'request>,
    // None when the ASGI scope carries no headers at all
    pub scope_headers: Option<&'request HeaderMap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaisedFailure {
    pub failure: ClientFailure,
    pub invalid_virtual_key: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error("the authentication failure was raised to the caller")]
    Rejected(Box<RaisedFailure>),
    #[error(transparent)]
    Hook(#[from] HookError),
    #[error("the failure identity could not be built: {0}")]
    Identity(#[from] IdentityError),
}

#[derive(Clone, Copy)]
struct Verdict {
    invalid_virtual_key: bool,
    quiet: bool,
}

pub struct AuthErrorHandler<'deps, Ports: ?Sized> {
    pub ports: &'deps Ports,
    pub settings: AuthErrorSettings,
    pub routes: &'deps RouteNormalizer,
    pub redactor: &'deps Redactor,
}

impl<Ports: AuthErrorPorts + ?Sized> AuthErrorHandler<'_, Ports> {
    // UserAPIKeyAuthExceptionHandler._handle_authentication_error
    pub async fn handle(
        &self,
        error: AuthFailure,
        request: FailedRequest<'_>,
    ) -> Result<UserAPIKeyAuth, HandlerError> {
        if let AuthFailure::Database(database_error) = &error
            && self.settings.allow_requests_on_db_unavailable
            && is_database_connection_error(database_error.error_type)
        {
            self.ports.record_service_failure(&ServiceFailure {
                error: database_error,
                call_type: "get_key_object",
                duration: Duration::ZERO,
            });
            self.ports.log(AuthLog {
                logger: Logger::Proxy,
                level: Level::Warning,
                message: OUTAGE_WARNING.to_owned(),
                traceback: false,
                requester_ip: None,
            });
            return outage_identity(request.route).map_err(HandlerError::Identity);
        }
        Err(self.reject(error, &request).await)
    }

    async fn reject(&self, original: AuthFailure, request: &FailedRequest<'_>) -> HandlerError {
        let requester_ip =
            request_ip_address(&request.provenance, self.settings.use_x_forwarded_for);
        // logged before identity seeding and callbacks, so a raising callback cannot lose the diagnostic
        let verdict = self.log_failure(&original, request, requester_ip);
        let identity = match self.failure_identity(request) {
            Ok(identity) => identity,
            Err(error) => return HandlerError::Identity(error),
        };
        self.ports
            .seed_identity(&identity, request.request_data.get("model"));
        let attributed = self.attribute_budget(original, request.request_data);
        let user_agent = request
            .scope_headers
            .and_then(|headers| headers.get(USER_AGENT))
            .map(|value| {
                value
                    .as_bytes()
                    .iter()
                    .map(|byte| char::from(*byte))
                    .collect::<String>()
            });
        let context = with_client_context(
            request.request_data,
            Some(requester_ip),
            user_agent.as_deref(),
        );
        let transformed = self
            .ports
            .post_call_failure_hook(FailureHookCall {
                request_data: &context,
                original: &attributed,
                identity: &identity,
                error_type: ProxyErrorTypes::AuthError,
                route: request.route,
            })
            .await;
        let raised = match transformed {
            Ok(replacement) => replacement.unwrap_or(attributed),
            Err(error) => return HandlerError::Hook(error),
        };
        self.raise(&raised, verdict, requester_ip)
    }

    fn log_failure(
        &self,
        original: &AuthFailure,
        request: &FailedRequest<'_>,
        requester_ip: &str,
    ) -> Verdict {
        let invalid_virtual_key = is_invalid_virtual_key_error(&original.marker_facts());
        let quiet = invalid_virtual_key && !self.settings.log.client_error_tracebacks;
        self.ports.log(AuthLog {
            logger: if quiet {
                Logger::ProxyStdout
            } else {
                Logger::Proxy
            },
            level: if quiet { Level::Warning } else { Level::Error },
            message: format!(
                "{FAILURE_PREFIX}{}\nRequester IP Address:{requester_ip}{}",
                self.redactor.redact_string(&original.to_string()),
                identity_log_suffix(request.resolved_identity, self.settings.log.key_identity),
            ),
            traceback: self.settings.log.client_error_tracebacks
                || !is_expected_client_error(Some(&original.client_error_facts())),
            requester_ip: Some(requester_ip.to_owned()),
        });
        Verdict {
            invalid_virtual_key,
            quiet,
        }
    }

    fn raise(&self, raised: &AuthFailure, verdict: Verdict, requester_ip: &str) -> HandlerError {
        let failure = as_proxy_exception(raised);
        let marked =
            mark_invalid_virtual_key_error(failure.exception(), verdict.invalid_virtual_key);
        // a quiet malformed-key warning that a callback turned into another status still needs an error line
        if verdict.quiet && failure.exception().code != "401" {
            self.ports.log(AuthLog {
                logger: Logger::Proxy,
                level: Level::Error,
                message: format!(
                    "{FAILURE_PREFIX}{}\nRequester IP Address:{requester_ip}",
                    self.redactor.redact_string(&failure.exception().message),
                ),
                traceback: false,
                requester_ip: Some(requester_ip.to_owned()),
            });
        }
        HandlerError::Rejected(Box::new(RaisedFailure {
            invalid_virtual_key: matches!(marked, MarkOutcome::Marked(_)),
            failure,
        }))
    }

    fn failure_identity(
        &self,
        request: &FailedRequest<'_>,
    ) -> Result<UserAPIKeyAuth, IdentityError> {
        let mut identity = if let Some(resolved) = request.resolved_identity {
            resolved.clone()
        } else {
            UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))?
        };
        identity
            .extra
            .parent_otel_span
            .clone_from(&request.parent_span);
        identity.extra.request_route = Some(self.routes.normalize_request_route(request.route));
        let has_key = identity
            .extra
            .api_key
            .0
            .as_ref()
            .is_some_and(|key| !key.expose_secret().is_empty());
        if !has_key {
            let hashed = safe_hash_api_key(request.api_key.expose_secret());
            identity.extra.api_key = ApiKeySecret(Some(SecretString::from(hashed)));
        }
        Ok(identity)
    }

    // tenant budget checks never see the request model, so their rejection carries no provider until here
    fn attribute_budget(&self, error: AuthFailure, request_data: &JsonObject) -> AuthFailure {
        let AuthFailure::BudgetExceeded {
            message,
            status_code,
            llm_provider,
        } = error
        else {
            return error;
        };
        if !llm_provider.is_empty() {
            return AuthFailure::BudgetExceeded {
                message,
                status_code,
                llm_provider,
            };
        }
        let model = match request_data.get("model") {
            Some(Json::Text(text)) => Some(text.as_str()),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            )
            | None => None,
        };
        let (_, provider) =
            resolve_rate_limit_provider(model, self.ports.native_resolution(), self.ports.router());
        AuthFailure::BudgetExceeded {
            message,
            status_code,
            llm_provider: provider,
        }
    }
}

// a restricted internal user, so an outage never hands an anonymous caller administrator grants
fn outage_identity(route: &str) -> Result<UserAPIKeyAuth, IdentityError> {
    let mut identity = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new()))?;
    identity.view.token.key_name = Some(OUTAGE_KEY_NAME.to_owned());
    identity.view.token.token = Some(SecretString::from(OUTAGE_KEY_NAME.to_owned()));
    identity.view.token.user_id = Some(DB_UNAVAILABLE_FALLBACK_USER_ID.to_owned());
    identity.extra.user_role = Some(UserRole::InternalUser);
    identity.extra.request_route = Some(route.to_owned());
    Ok(identity)
}

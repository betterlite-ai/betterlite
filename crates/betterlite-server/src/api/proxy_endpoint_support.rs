use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::database_faults::{DbError, is_read_only_transaction_error};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::structured::SensitiveDataMasker;
use betterlite_reporting::bug_report::notice::strip_bug_report_notice;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use tokio_util::task::TaskTracker;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

mod tests;

const LITELLM_CALL_ID_HEADER: &str = "x-litellm-call-id";
const INTERNAL_SERVER_ERROR: &str = "internal_server_error";
const STATUS_SERVER_ERROR: u16 = 500;
const STRINGIFIED_NONE: &str = "None";
const CREDENTIAL_FIELDS: [&str; 6] = [
    "api_key",
    "client_secret",
    "vertex_credentials",
    "vertex_ai_credentials",
    "aws_access_key_id",
    "aws_secret_access_key",
];
const RATE_LIMIT_FIELDS: [&str; 2] = ["default_api_key_tpm_limit", "default_api_key_rpm_limit"];
// LiteLLM's DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER
const MASK_DEPTH: usize = 10;
pub const READ_ONLY_RECOVERY_REASON: &str = "postgres_read_only_transaction";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpFailure {
    pub status_code: u16,
    pub detail: String,
}

// getattr(e, ...) reads HTTPException.detail and HTTPException.status_code; the param stays nullable
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureFacts<'caught> {
    pub message: &'caught str,
    pub exception_type: &'caught str,
    pub status_code: Option<u16>,
    pub param: Option<&'caught str>,
    pub database_error: Option<&'caught DbError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaughtException<'caught> {
    Http(HttpFailure),
    Proxy(ProxyException),
    Failure(FailureFacts<'caught>),
}

// B4170's should_report_bug and bug_report_notice, injected so this module never reads the environment itself
pub trait ProxyDiagnostics: Send + Sync {
    fn bug_report_notice(&self, failure: &FailureFacts<'_>) -> Option<String>;
}

// B8396's ReadOnlyWriter::recreate, injected so this module schedules the recovery without owning the database
pub trait ReadOnlyWriterRecovery: Send + Sync + 'static {
    fn recreate_read_only_writer(&self, reason: &str) -> impl Future<Output = ()> + Send;
}

// proxy_logging_obj.recreate: enqueue without awaiting, the server's tracker joins the work at shutdown
pub trait RecoverySubmission: Send + Sync {
    fn submit_read_only_recovery(&self);
}

pub struct TrackedRecovery<Writer> {
    pub tracker: TaskTracker,
    pub writer: Arc<Writer>,
}

impl<Writer> RecoverySubmission for TrackedRecovery<Writer>
where
    Writer: ReadOnlyWriterRecovery,
{
    fn submit_read_only_recovery(&self) {
        let writer = Arc::clone(&self.writer);
        let _task = self.tracker.spawn(async move {
            writer
                .recreate_read_only_writer(READ_ONLY_RECOVERY_REASON)
                .await;
        });
    }
}

// _PROXY_MaxParallelRequestsHandler_v3 is the only handler that answers async_release_max_parallel_requests_on_disconnect
pub trait DisconnectRelease: Send + Sync {
    type Failure: Error + Send + 'static;

    fn release_on_disconnect(
        &self,
        principal: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParallelLimiterHook<'hook, Handler> {
    V3(&'hook Handler),
    Older,
}

// proxy_logging_obj.get_proxy_hook("parallel_request_limiter"), which answers None when no hook was registered
pub trait ParallelLimiterHooks: Send + Sync {
    type Handler: DisconnectRelease;

    fn parallel_request_limiter(&self) -> Option<ParallelLimiterHook<'_, Self::Handler>>;
}

// Router.get_model_names, injected so the upload route can answer the exact-name predicate from its catalog
pub trait ModelNameCatalog {
    fn model_names(&self) -> Result<Vec<String>, LookupError>;
}

impl ModelNameCatalog for CatalogLookup<'_> {
    fn model_names(&self) -> Result<Vec<String>, LookupError> {
        // the upload predicate asks the bare name, so team-specific public names stay out of the set
        self.get_model_names(None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RedactError {
    #[error("the deployment has no litellm_params object to mask")]
    MissingParams,
    #[error("the deployment's litellm_params could not be read as JSON")]
    Json(#[source] serde_json::Error),
}

#[must_use]
pub fn handle_exception_on_proxy(
    exception: CaughtException<'_>,
    litellm_call_id: Option<&str>,
    diagnostics: &dyn ProxyDiagnostics,
    recovery: Option<&dyn RecoverySubmission>,
) -> ProxyException {
    match &exception {
        CaughtException::Http(http) => {
            tracing::error!("Exception: {}: {}", http.status_code, http.detail);
        }
        CaughtException::Proxy(proxy) => tracing::error!("Exception: {}", proxy.message),
        CaughtException::Failure(facts) => tracing::error!("Exception: {}", facts.message),
    }
    if let CaughtException::Failure(facts) = &exception
        && let Some(database_error) = facts.database_error
        && is_read_only_transaction_error(database_error)
        && let Some(submission) = recovery
    {
        submission.submit_read_only_recovery();
    }
    let headers = litellm_call_id_headers(litellm_call_id);
    match exception {
        CaughtException::Http(http) => ProxyException::new(ProxyExceptionInit {
            message: http.detail,
            error_type: INTERNAL_SERVER_ERROR.to_owned(),
            param: None,
            code: ExceptionCode::Status(i64::from(http.status_code)),
            headers,
            openai_code: None,
            provider_specific_fields: BTreeMap::new(),
        }),
        CaughtException::Proxy(proxy) => with_litellm_call_id(proxy, litellm_call_id),
        CaughtException::Failure(facts) => failure_record(&facts, headers, diagnostics),
    }
}

fn failure_record(
    facts: &FailureFacts<'_>,
    headers: BTreeMap<String, String>,
    diagnostics: &dyn ProxyDiagnostics,
) -> ProxyException {
    if let Some(notice) = diagnostics.bug_report_notice(facts) {
        tracing::error!("{notice}");
    }
    ProxyException::new(ProxyExceptionInit {
        message: strip_bug_report_notice(facts.message).to_owned(),
        error_type: INTERNAL_SERVER_ERROR.to_owned(),
        param: facts
            .param
            .filter(|param| *param != STRINGIFIED_NONE)
            .map(str::to_owned),
        code: ExceptionCode::Status(i64::from(facts.status_code.unwrap_or(STATUS_SERVER_ERROR))),
        headers,
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn litellm_call_id_headers(litellm_call_id: Option<&str>) -> BTreeMap<String, String> {
    let Some(id) = litellm_call_id else {
        return BTreeMap::new();
    };
    BTreeMap::from([(LITELLM_CALL_ID_HEADER.to_owned(), id.to_owned())])
}

fn with_litellm_call_id(
    mut exception: ProxyException,
    litellm_call_id: Option<&str>,
) -> ProxyException {
    if let Some(id) = litellm_call_id {
        exception
            .headers
            .entry(LITELLM_CALL_ID_HEADER.to_owned())
            .or_insert_with(|| id.to_owned());
    }
    exception
}

// Router.get_model_names: the upload predicate only matches a name the catalog lists exactly
pub fn is_known_model(
    model: Option<&str>,
    router: Option<&dyn ModelNameCatalog>,
) -> Result<bool, LookupError> {
    let (Some(name), Some(catalog)) = (model, router) else {
        return Ok(false);
    };
    let names = catalog.model_names()?;
    Ok(names.iter().any(|known| known == name))
}

// ProxyLogging._arelease_max_parallel_requests_on_disconnect: only the v3 handler is awaited, never a retry
pub async fn release_parallel_slot<Hooks>(
    hooks: &Hooks,
    principal: &UserAPIKeyAuth,
) -> Result<(), <Hooks::Handler as DisconnectRelease>::Failure>
where
    Hooks: ParallelLimiterHooks,
{
    let Some(ParallelLimiterHook::V3(limiter)) = hooks.parallel_request_limiter() else {
        return Ok(());
    };
    limiter.release_on_disconnect(principal).await
}

// remove_sensitive_info_from_deployment: the credentials leave before the masker sees the rest
pub fn redact_deployment(
    mut deployment: JsonObject,
    excluded_keys: &[String],
) -> Result<JsonObject, RedactError> {
    let Some(Json::Object(mut params)) = deployment.shift_remove("litellm_params") else {
        return Err(RedactError::MissingParams);
    };
    for field in CREDENTIAL_FIELDS {
        params.shift_remove(field);
    }
    let payload = serde_json::to_value(Json::Object(params)).map_err(RedactError::Json)?;
    let mapping = payload.as_object().ok_or(RedactError::MissingParams)?;
    let mut excluded = excluded_keys.to_vec();
    excluded.extend(RATE_LIMIT_FIELDS.map(str::to_owned));
    let masked = SensitiveDataMasker::default().mask_dict(mapping, 0, MASK_DEPTH, &excluded);
    let masked_params = serde_json::from_value::<Json>(masked).map_err(RedactError::Json)?;
    deployment.insert("litellm_params".to_owned(), masked_params);
    Ok(deployment)
}

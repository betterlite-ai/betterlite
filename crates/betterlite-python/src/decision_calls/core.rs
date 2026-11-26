use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Instant as OidcInstant;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_foundry::responses::decisions::AzureAiDecisionsConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_provider::e2b::E2bSandbox;
use betterlite_provider::e2b::transport::ProxiedSandboxHttp;
use betterlite_provider::opensandbox::helpers::{MonotonicClock, OpenSandbox};
use betterlite_provider::sandbox_dispatch::lifecycle::SandboxDispatch;
use betterlite_provider::sandbox_dispatch::registry::SandboxToolRegistry;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;
use tokio::time::Instant;

use self::asynchronous::adecision_call;
use self::decisions::{DecisionFormat, sync_decision};
use self::sandbox::{
    code_interpreter, create_sandbox, delete_sandbox, register_tools, resolve_tool, run_code,
};
use super::convert::{CallWire, ConvertError, SessionWire, parse};
use super::error::{SessionError, SessionFailure};
use super::transport::ProxiedOpenSandboxHttp;
use crate::completion::setup::{outbound_client, runtime_config};

mod asynchronous;
mod decisions;
mod sandbox;

pub(crate) const DECISIONS: &str = "decisions";
pub(crate) const SYSTEMONE: &str = "systemone";
pub(crate) const CREATE: &str = "create_sandbox";
pub(crate) const RUN: &str = "run_code";
pub(crate) const DELETE: &str = "delete_sandbox";
pub(crate) const CODE_INTERPRETER: &str = "code_interpreter";
pub(crate) const REGISTER: &str = "register_tools";
pub(crate) const RESOLVE: &str = "resolve_tool";

pub(crate) type Sandbox = SandboxDispatch<
    E2bSandbox<ProxiedSandboxHttp, BTreeMap<String, String>>,
    OpenSandbox<ProxiedOpenSandboxHttp, BTreeMap<String, String>, MonotonicClock>,
>;

// Everything one Python session owns, plus the sandbox tool registry the sandbox_tools module keeps here.
pub(crate) struct SessionCore {
    environment: BTreeMap<String, String>,
    catalog: Arc<ModelCatalog>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    registry: JSONProviderRegistry,
    tls: TlsPolicy,
    proxy: Arc<ProxyPolicy>,
    redactor: Redactor,
    sink: SilentSink,
    client: ProxiedClient,
    tools: Mutex<SandboxToolRegistry>,
}

impl SessionCore {
    pub(crate) fn build(text: &str) -> Result<Self, SessionError> {
        let wire: SessionWire = parse(text)?;
        let environment = wire.environment.unwrap_or_else(process_environment);
        let config = runtime_config();
        let tls = TlsPolicy::resolve(&config, &environment, None)
            .map_err(|source| SessionError::Tls { source })?;
        let proxy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(|source| SessionError::Proxy { source })?;
        let client = outbound_client(&environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redactor { source })?;
        Ok(Self {
            environment,
            catalog: BundledCatalog::default().get(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            tls,
            proxy: Arc::new(proxy),
            redactor,
            sink: SilentSink,
            client,
            tools: Mutex::new(SandboxToolRegistry::default()),
        })
    }

    pub(crate) fn call(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        match wire.operation.as_str() {
            DECISIONS => sync_decision(self, wire, DecisionFormat::Decisions),
            SYSTEMONE => sync_decision(self, wire, DecisionFormat::SystemOne),
            REGISTER => register_tools(self, wire),
            RESOLVE => resolve_tool(self, wire),
            other => Err(SessionFailure::from(ConvertError::Operation {
                operation: other.to_owned(),
            })),
        }
    }

    pub(crate) async fn acall(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        match wire.operation.as_str() {
            DECISIONS => adecision_call(self, wire, DecisionFormat::Decisions).await,
            SYSTEMONE => adecision_call(self, wire, DecisionFormat::SystemOne).await,
            CREATE => create_sandbox(self, wire).await,
            RUN => run_code(self, wire).await,
            DELETE => delete_sandbox(self, wire).await,
            CODE_INTERPRETER => code_interpreter(self, wire).await,
            other => Err(SessionFailure::from(ConvertError::Operation {
                operation: other.to_owned(),
            })),
        }
    }

    fn resolution(&self, globals: &GlobalCredentials) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: globals.clone(),
        }
    }

    fn pre_post_env(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: true,
                key_info: true,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: chrono::Utc::now().fixed_offset(),
        }
    }

    fn sandbox(&self) -> Sandbox {
        SandboxDispatch {
            e2b: E2bSandbox {
                http: ProxiedSandboxHttp {
                    client: self.client.clone(),
                },
                environment: self.environment.clone(),
            },
            opensandbox: OpenSandbox {
                http: ProxiedOpenSandboxHttp {
                    client: self.client.clone(),
                },
                environment: self.environment.clone(),
                clock: MonotonicClock {
                    origin: Instant::now(),
                },
            },
        }
    }
}

// betterlite-foundry owns the Azure AI route builder and depends on betterlite-openai, so the closure arrives here
fn azure_ai() -> AzureAiConfig {
    AzureAiConfig {
        complete_url: Box::new(|api_base: &str, model: &str| {
            AzureAiDecisionsConfig.get_complete_url(api_base, model)
        }),
    }
}

// none of the manager, OIDC or Azure paths are reachable from a sandbox tool's os.environ/ reference
fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidcHttp),
        azure_tokens: Arc::new(NoAzureTokens),
        clock: Arc::new(FixedClock),
        oidc_cache: OidcTokenCache::default(),
    }
}

// The Python call has no callbacks to hand logging records on.
struct SilentSink;

impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}

#[cfg_attr(test, mutants::skip)]
struct NoOidcHttp;

#[cfg_attr(test, mutants::skip)]
impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("a Python session has no OIDC provider"),
        })
    }
}

#[cfg_attr(test, mutants::skip)]
struct NoAzureTokens;

#[cfg_attr(test, mutants::skip)]
impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

#[cfg_attr(test, mutants::skip)]
struct FixedClock;

#[cfg_attr(test, mutants::skip)]
impl OidcClock for FixedClock {
    fn now(&self) -> OidcInstant {
        OidcInstant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

mod tests;

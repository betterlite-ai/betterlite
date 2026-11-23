use super::capture::ErrorBodyCapture;
use super::client::ClientState;
use super::convert::{
    ClientWire, McpErrorKind, NativeMcpErrorPayload, ResolvedSettings, resolve_settings,
};
use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::SharedTimeSource;
use aws_smithy_runtime_api::client::http::SharedHttpClient;
use aws_types::os_shim_internal::Fs;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, VerifySetting};
use betterlite_config::types::{Config, GeneralSettings, LiteLlmSettings, ProxyTrust};
use betterlite_http::aws_duplex::AwsDuplexClient;
use betterlite_http::mcp_sigv4::{AwsSources, McpSigV4Auth, McpSigV4Settings};
use betterlite_http::proxy::ProxiedClient;
use betterlite_mcp::client::http_transport::send::HttpBase;
use betterlite_mcp::client::sdk_runtime::runtime::SdkRuntime;
use betterlite_mcp::client::sessions::stdio::SDK_SHUTDOWN;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use pyo3::{PyErr, PyResult, Python};
use pyo3_async_runtimes::tokio::get_runtime;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::task::TaskTracker;
const CREDENTIALS_TIMEOUT_SECONDS: u64 = 60;
mod tests;
pub(crate) fn build(py: Python<'_>, wire: ClientWire) -> PyResult<ClientState> {
    let environment = process_environment();
    let mut resolved = resolve_settings(wire, &environment).map_err(PyErr::from)?;
    let tls = tls_policy(resolved.ssl_verify.clone(), &environment).map_err(PyErr::from)?;
    let proxy = proxy_policy(&environment).map_err(PyErr::from)?;
    let aws = aws_credentials(py, &mut resolved, &proxy, &tls)?;
    let runtime = SdkRuntime::new(
        HttpBase {
            proxy,
            tls: tls.clone(),
        },
        Arc::new(ErrorBodyCapture),
        TaskTracker::new(),
        SDK_SHUTDOWN,
    );
    Ok(client_state(resolved, tls, aws, runtime))
}
fn aws_credentials(
    py: Python<'_>,
    resolved: &mut ResolvedSettings,
    proxy: &Arc<ProxyPolicy>,
    tls: &TlsPolicy,
) -> PyResult<Option<Arc<McpSigV4Auth>>> {
    let Some(settings) = resolved.aws.take() else {
        return Ok(None);
    };
    let built = py
        .detach(|| get_runtime().block_on(aws_signer(settings, proxy, tls, &resolved.environment)));
    let signer = built.map_err(PyErr::from)?;
    Ok(Some(Arc::new(signer)))
}
fn client_state(
    resolved: ResolvedSettings,
    tls: TlsPolicy,
    aws: Option<Arc<McpSigV4Auth>>,
    runtime: SdkRuntime,
) -> ClientState {
    ClientState {
        server_url: resolved.server_url,
        transport_type: resolved.transport_type,
        stdio: resolved.stdio,
        environment: resolved.environment,
        npm_cache_dir: resolved.npm_cache_dir,
        tls,
        protocol_version: resolved.protocol_version,
        auth_type: resolved.auth_type,
        auth_header_name: resolved.auth_header_name,
        extra_headers: resolved.extra_headers,
        timeout: resolved.timeout,
        listing_timeout: resolved.listing_timeout,
        auth_value: Mutex::new(resolved.auth_value),
        aws,
        runtime: Arc::new(runtime),
        instructions: Mutex::new(None),
        tracker: TaskTracker::new(),
    }
}
fn proxy_policy(
    environment: &BTreeMap<String, String>,
) -> Result<Arc<ProxyPolicy>, NativeMcpErrorPayload> {
    ProxyPolicy::resolve(&ProxyTrust::default(), environment)
        .map(Arc::new)
        .map_err(|error| NativeMcpErrorPayload::plain(McpErrorKind::ValueError, error.to_string()))
}
// An explicit ssl_verify is what LiteLLM's get_ssl_verify answers, so it outranks SSL_VERIFY here
fn tls_policy(
    explicit: Option<VerifySetting>,
    environment: &BTreeMap<String, String>,
) -> Result<TlsPolicy, NativeMcpErrorPayload> {
    let mut config = tls_config();
    config.litellm_settings.ssl_verify = explicit.clone();
    let resolved_environment = if explicit.is_some() {
        let mut filtered = environment.clone();
        filtered.remove("SSL_VERIFY");
        filtered
    } else {
        environment.clone()
    };
    TlsPolicy::resolve(&config, &resolved_environment, None)
        .map_err(|error| NativeMcpErrorPayload::plain(McpErrorKind::ValueError, error.to_string()))
}
async fn aws_signer(
    settings: McpSigV4Settings,
    proxy: &Arc<ProxyPolicy>,
    tls: &TlsPolicy,
    environment: &BTreeMap<String, String>,
) -> Result<McpSigV4Auth, NativeMcpErrorPayload> {
    let client = ProxiedClient::outbound_with_tls(proxy, tls).map_err(|error| {
        NativeMcpErrorPayload::plain(McpErrorKind::ValueError, error.to_string())
    })?;
    let sources = AwsSources {
        environment: environment.clone().into_iter().collect(),
        filesystem: Fs::real(),
        client: SharedHttpClient::new(AwsDuplexClient::from_proxied(&client)),
        clock: SharedTimeSource::default(),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(CREDENTIALS_TIMEOUT_SECONDS),
    };
    McpSigV4Auth::new(settings, sources)
        .await
        .map_err(|error| NativeMcpErrorPayload::plain(McpErrorKind::ValueError, error.to_string()))
}
fn tls_config() -> Config {
    Config {
        model_list: Vec::new(),
        general_settings: Box::new(GeneralSettings {
            cipher: CipherSettings {
                signing_key: None,
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            master_key: None,
        }),
        router_settings: Box::default(),
        litellm_settings: LiteLlmSettings {
            ssl_verify: None,
            ssl_certificate: None,
            http2: None,
            ssl_ecdh_curve: None,
            ssl_security_level: None,
            num_retries: None,
            request_timeout: None,
            drop_params: None,
            gemini_live_defer_setup: false,
            proxy_trust: ProxyTrust::default(),
            bedrock_metadata_fields: Vec::new(),
            modify_params: false,
            default_max_tokens: 0,
        },
        environment_variables: BTreeMap::new(),
        sections: Box::default(),
    }
}

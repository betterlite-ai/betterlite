use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::ProxiedClient;

use crate::registry::catalog::{CatalogError, CatalogInputs};
use crate::s3_batch::flush::SystemMonotonic;
use crate::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
use crate::s3_transport::option_value::OptionValue;
use crate::s3_transport::signing::SystemClock;
use crate::s3_transport::transport::HttpS3Client;
use crate::s3_v2::parameters::resolve_s3_parameters;
use crate::s3_v2::source::Source;

const TLS_VARIABLES: [&str; 7] = [
    "SSL_VERIFY",
    "SSL_SECURITY_LEVEL",
    "SSL_CERT_FILE",
    "SSL_CERTIFICATE",
    "LITELLM_HTTP2",
    "SSL_ECDH_CURVE",
    "LITELLM_USER_AGENT",
];

// LiteLLM's S3Logger(s3_callback_params_override=litellm.s3_audit_callback_params): its own queue, limiter, client and flush loop
pub fn build_audit_logger(
    inputs: &CatalogInputs,
    audit_source: &Source<()>,
) -> Result<Arc<S3BatchLogger>, CatalogError> {
    tracing::debug!("in init s3 logger (audit override) - {audit_source:?}");
    let defaults = Source::<()>::default();
    // the resolver reads the audit source in place of the global one, so s3_callback_params never reaches this logger
    let parameters = resolve_s3_parameters(
        &defaults,
        &defaults,
        Some(audit_source),
        inputs.env.as_ref(),
    );
    let http = audit_client(inputs, parameters.connection.verify.as_ref())?;
    S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client: Arc::new(HttpS3Client { client: http }),
            clock: Arc::new(SystemClock),
            monotonic: Arc::new(SystemMonotonic::default()),
            environment: Arc::clone(&inputs.env),
            throttle_observer: None,
            cold_storage_logger: None,
            tasks: Some(inputs.tracker.clone()),
            shutdown: inputs.shutdown.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })
}

// s3_verify overrides the proxy-wide ssl_verify for the audit logger's own client, as LiteLLM's ssl_verify param does
fn audit_client(
    inputs: &CatalogInputs,
    verify: Option<&OptionValue>,
) -> Result<ProxiedClient, CatalogError> {
    let Some(value) = verify else {
        return Ok(inputs.http.clone());
    };
    let Some(enabled) = value.as_bool() else {
        let found = value.type_name();
        let message = format!("s3_audit_callback_params.s3_verify: expected a bool, got {found}");
        return Err(CatalogError::Init { message });
    };
    let policy = TlsPolicy::resolve(&inputs.config, &tls_variables(inputs), Some(enabled))
        .map_err(|error| CatalogError::Init {
            message: error.to_string(),
        })?;
    ProxiedClient::outbound_with_tls(&inputs.http.policy, &policy).map_err(|error| {
        CatalogError::Init {
            message: error.to_string(),
        }
    })
}

fn tls_variables(inputs: &CatalogInputs) -> BTreeMap<String, String> {
    let mut variables = BTreeMap::new();
    for name in TLS_VARIABLES {
        if let Some(value) = inputs.env.lookup(name) {
            variables.insert(name.to_owned(), value);
        }
    }
    variables
}

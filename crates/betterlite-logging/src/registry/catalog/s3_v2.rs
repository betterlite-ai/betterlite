use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::s3_batch::flush::SystemMonotonic;
use crate::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
use crate::s3_transport::option_value::OptionValue;
use crate::s3_transport::signing::SystemClock;
use crate::s3_transport::transport::HttpS3Client;
use crate::s3_v2::parameters::resolve_s3_parameters;
use crate::s3_v2::source::Source;
use betterlite_config::sections::callback_settings::names::CallbackName;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::ProxiedClient;
use std::collections::BTreeMap;
use std::sync::Arc;
mod tests;
const TLS_VARIABLES: [&str; 7] = [
    "SSL_VERIFY",
    "SSL_SECURITY_LEVEL",
    "SSL_CERT_FILE",
    "SSL_CERTIFICATE",
    "LITELLM_HTTP2",
    "SSL_ECDH_CURVE",
    "LITELLM_USER_AGENT",
];
pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let source = parameter_source(inputs)?;
    let parameters =
        resolve_s3_parameters::<()>(&Source::default(), &source, None, inputs.env.as_ref());
    let http = match tls_policy(inputs, parameters.connection.verify.as_ref())? {
        Some(tls) => {
            ProxiedClient::outbound_with_tls(&inputs.http.policy, &tls).map_err(|error| {
                CatalogError::Init {
                    message: error.to_string(),
                }
            })?
        }
        None => inputs.http.clone(),
    };
    let logger: Arc<dyn CallbackLogger> = S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client: Arc::new(HttpS3Client { client: http }),
            clock: Arc::new(SystemClock),
            monotonic: Arc::new(SystemMonotonic::default()),
            environment: Arc::clone(&inputs.env),
            throttle_observer: None,
            // B1729 loads litellm_settings.cold_storage_custom_logger; until then no logger is cold storage
            cold_storage_logger: None,
            tasks: Some(inputs.tracker.clone()),
            shutdown: inputs.shutdown.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    Ok(logger)
}
// s3_verify overrides the proxy-wide ssl_verify for this logger's client, as LiteLLM's ssl_verify param does
fn tls_policy(
    inputs: &CatalogInputs,
    verify: Option<&OptionValue>,
) -> Result<Option<TlsPolicy>, CatalogError> {
    let Some(value) = verify else {
        return Ok(None);
    };
    let Some(enabled) = value.as_bool() else {
        let found = value.type_name();
        let message = format!("callback_settings.s3_v2.s3_verify: expected a bool, got {found}");
        return Err(CatalogError::Init { message });
    };
    let environment: BTreeMap<String, String> = TLS_VARIABLES
        .iter()
        .filter_map(|name| Some(((*name).to_owned(), inputs.env.lookup(name)?)))
        .collect();
    TlsPolicy::resolve(&inputs.config, &environment, Some(enabled))
        .map(Some)
        .map_err(|error| CatalogError::Init {
            message: error.to_string(),
        })
}
// the scalar keys a proxy config keeps under callback_settings.s3_v2 carry the s3_callback_params bag
fn parameter_source(inputs: &CatalogInputs) -> Result<Source<()>, CatalogError> {
    let name = CallbackName::S3V2.as_str();
    let Some(entry) = inputs.callback_settings.get(name) else {
        return Ok(Source::default());
    };
    let text =
        serde_json::to_string(&entry.exposed_variables()).map_err(|source| CatalogError::Init {
            message: source.to_string(),
        })?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| CatalogError::Init {
        message: format!(
            "callback_settings.{name}.{}: {}",
            error.path(),
            error.inner()
        ),
    })
}

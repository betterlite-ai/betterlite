use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::langfuse_otel::LangfuseOtelLogger;
use betterlite_logging::langfuse_otel::destination::{
    build_langfuse_otel_config, build_langfuse_otel_headers, format_otel_headers,
    get_langfuse_otel_config, langfuse_authorization_header, langfuse_otel_host,
};
use betterlite_logging::langfuse_otel::dynamic::{
    construct_dynamic_otel_config, construct_dynamic_otel_headers,
};
use betterlite_logging::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig};
use betterlite_logging::otel::plumbing::exporters::HttpClients;
use betterlite_logging::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
};
use betterlite_logging::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use betterlite_logging::otel_provider_assembly::proxy_registry::ProxyRegistry;
use betterlite_models::decisions::Json;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::InMemorySpanExporter;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Initialization {
        environment: BTreeMap<String, String>,
    },
    Host {
        environment: BTreeMap<String, String>,
    },
    GetConfig {
        environment: BTreeMap<String, String>,
    },
    BuildConfig {
        environment: BTreeMap<String, String>,
        public_key: String,
        secret_key: SecretString,
        langfuse_host: Option<String>,
    },
    AuthorizationHeader {
        public_key: String,
        secret_key: SecretString,
    },
    BuildHeaders {
        auth_header: SecretString,
    },
    FormatHeaders {
        auth_header: SecretString,
    },
    DynamicHeaders {
        params: Json,
    },
    DynamicConfig {
        environment: BTreeMap<String, String>,
        params: Json,
    },
}

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no connection is opened by this helper"))
    }
}

fn config_json(config: &OpenTelemetryConfig) -> Value {
    let exporter = match &config.exporter {
        Exporter::Console => "console",
        Exporter::InMemory => "in_memory",
        Exporter::Named(name) => name.as_str(),
    };
    json!({
        "exporter": exporter,
        "endpoint": config.endpoint,
        "headers": config.headers,
        "service_name": config.service_name,
        "deployment_environment": config.deployment_environment,
        "model_id": config.model_id,
        "enable_metrics": config.signals.enable_metrics,
        "enable_events": config.signals.enable_events,
    })
}

fn header_pairs(headers: &betterlite_logging::langfuse_otel::destination::OtelHeaders) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| json!([name, value.expose_secret()]))
            .collect(),
    )
}

fn initialized_config(environment: &BTreeMap<String, String>) -> HelperResult {
    let client: Arc<dyn HttpClient> = Arc::new(NoServer);
    let globals = GlobalProviders::default();
    let proxy = ProxyRegistry::default();
    let init = LegacyOtelInit {
        config: None,
        callback_name: Some("langfuse_otel".to_owned()),
        overrides: MetadataKeyOverrides {
            team: None,
            metadata: None,
        },
        injected: InjectedProviders {
            tracer: None,
            meter: None,
            logger: None,
        },
        max_dynamic_tracer_providers: MAX_DYNAMIC_TRACER_PROVIDERS,
    };
    let logger = LangfuseOtelLogger::new(
        init,
        &LegacyOtelEnvironment {
            env: environment,
            clients: HttpClients {
                traces: Arc::clone(&client),
                metrics: Arc::clone(&client),
                logs: client,
            },
            shared_exporter: InMemorySpanExporter::default(),
            globals: &globals,
            proxy: Some(&proxy),
        },
    )
    .or_value_error()?;
    Ok(config_json(&logger.otel.config))
}

fn dynamic_config(params: &Json, environment: &BTreeMap<String, String>) -> HelperResult {
    Ok(construct_dynamic_otel_config(params, environment)
        .or_value_error()?
        .as_ref()
        .map_or(Value::Null, config_json))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Initialization { environment } => initialized_config(environment),
            Input::Host { environment } => Ok(json!(langfuse_otel_host(environment))),
            Input::GetConfig { environment } => Ok(config_json(
                &get_langfuse_otel_config(environment).or_value_error()?,
            )),
            Input::BuildConfig {
                environment,
                public_key,
                secret_key,
                langfuse_host,
            } => Ok(config_json(&build_langfuse_otel_config(
                public_key,
                secret_key,
                langfuse_host.as_deref(),
                environment,
            ))),
            Input::AuthorizationHeader {
                public_key,
                secret_key,
            } => Ok(json!(
                langfuse_authorization_header(public_key, secret_key).expose_secret()
            )),
            Input::BuildHeaders { auth_header } => Ok(header_pairs(&build_langfuse_otel_headers(
                auth_header.clone(),
            ))),
            Input::FormatHeaders { auth_header } => Ok(json!(format_otel_headers(
                &build_langfuse_otel_headers(auth_header.clone())
            ))),
            Input::DynamicHeaders { params } => Ok(header_pairs(
                &construct_dynamic_otel_headers(params).or_value_error()?,
            )),
            Input::DynamicConfig {
                environment,
                params,
            } => dynamic_config(params, environment),
        }
    }
}

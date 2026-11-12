#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::trace::InMemorySpanExporter;

use super::LangfuseOtelLogger;
use super::error::LangfuseOtelError;
use crate::otel::legacy_exporters::tracer_cache::MAX_DYNAMIC_TRACER_PROVIDERS;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel_provider_assembly::legacy_otel_init::{
    InjectedProviders, LegacyOtelEnvironment, LegacyOtelInit, MetadataKeyOverrides,
};
use crate::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use crate::otel_provider_assembly::proxy_registry::ProxyRegistry;

mod authorization_header;
mod config_creation;
mod config_validation;
mod dynamic_config;
mod dynamic_headers;
mod endpoint_construction;
mod header_construction;
mod header_formatting;
mod host_resolution;
mod initialization;
mod service_hook_noop;
mod service_hook_span;

const PUBLIC_KEY: &str = "pk-lf-1";
const SECRET_KEY: &str = "sk-lf-2";
// base64 of "pk-lf-1:sk-lf-2"
const BASIC_AUTH: &str = "Basic cGstbGYtMTpzay1sZi0y";

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no HTTP client is wired into this test"))
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(pairs: &[(&str, Json)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<IndexMap<String, Json>>(),
    )
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn langfuse_headers(authorization: &str) -> String {
    format!("Authorization={authorization},x-langfuse-ingestion-version=4")
}

struct Rig {
    env: BTreeMap<String, String>,
    globals: GlobalProviders,
    proxy: ProxyRegistry,
    exporter: InMemorySpanExporter,
}

impl Rig {
    fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            env: environment(pairs),
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            exporter: InMemorySpanExporter::default(),
        }
    }

    fn logger(
        &self,
        config: Option<OpenTelemetryConfig>,
    ) -> Result<LangfuseOtelLogger, LangfuseOtelError> {
        let client: Arc<dyn HttpClient> = Arc::new(NoServer);
        let init = LegacyOtelInit {
            config,
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
        LangfuseOtelLogger::new(
            init,
            &LegacyOtelEnvironment {
                env: &self.env,
                clients: HttpClients {
                    traces: Arc::clone(&client),
                    metrics: Arc::clone(&client),
                    logs: client,
                },
                shared_exporter: self.exporter.clone(),
                globals: &self.globals,
                proxy: Some(&self.proxy),
            },
        )
    }
}

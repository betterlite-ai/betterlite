use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::realtime_contracts::config::{
    BaseRealtimeHttpConfig, Environment, RealtimeUrl,
};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_models::decisions::Json;
use betterlite_provider::api::realtime_http_transport::client::RealtimeClientSource;
use thiserror::Error;

use super::super::error::SdkHttpError;

pub(super) const REFUSED: &str = "the echo config refuses this model";

// The pooled client both handlers reach: the case env only carries the proxy settings and the TLS policy comes from an empty config
pub(super) struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
}

impl RealtimeClientSource for Source {
    fn client(&self) -> Result<ProxiedClient, ClientError> {
        self.pool
            .client(&self.tls, &self.routing)
            .map_err(ClientError::Pool)
    }
}

pub(super) fn source(
    config: &Config,
    env: &BTreeMap<String, String>,
) -> Result<Source, SdkHttpError> {
    let routing = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let tls = TlsPolicy::resolve(config, env, None)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    Ok(Source {
        pool: ClientPool::new(1, TransportLimits::DEFAULT),
        tls,
        routing,
    })
}

// The config's own error: a ValueError on the Python side, which the handler lets escape before its send
#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct Refused {
    pub(super) message: String,
}

// A provider config that echoes its inputs into recognizable URLs, so a case shows which member the handler called
pub(super) struct AdapterConfig;

impl BaseRealtimeHttpConfig for AdapterConfig {
    type Error = Refused;

    fn get_api_base(&self, api_base: Option<&str>) -> Result<String, Refused> {
        Ok(api_base.unwrap_or_default().to_owned())
    }

    fn get_api_key(&self, api_key: Option<&str>) -> Result<String, Refused> {
        Ok(api_key.unwrap_or_default().to_owned())
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        api_version: Option<&str>,
    ) -> String {
        format!(
            "{}/custom/{model}/{}",
            api_base.unwrap_or_default(),
            api_version.unwrap_or("none")
        )
    }

    fn get_transcription_session_url(&self, url: &RealtimeUrl<'_>) -> String {
        format!(
            "{}/transcription/{}/{}",
            url.api_base.unwrap_or_default(),
            url.model,
            url.api_version.unwrap_or("none")
        )
    }

    fn get_realtime_calls_url(&self, url: &RealtimeUrl<'_>) -> String {
        format!(
            "{}/calls/{}/{}",
            url.api_base.unwrap_or_default().trim_end_matches('/'),
            url.model,
            url.api_version.unwrap_or("none")
        )
    }

    fn get_realtime_calls_headers(&self, ephemeral_key: &str) -> HeaderMap {
        HeaderMap::from([(
            String::from("x-ephemeral"),
            Json::Text(ephemeral_key.to_owned()),
        )])
    }

    fn validate_environment(&self, environment: &Environment<'_>) -> Result<HeaderMap, Refused> {
        if environment.model == "refuse" {
            return Err(Refused {
                message: String::from(REFUSED),
            });
        }
        Ok(HeaderMap::from([
            (
                String::from("api-key"),
                Json::Text(environment.api_key.unwrap_or_default().to_owned()),
            ),
            (
                String::from("Content-Type"),
                Json::Text(String::from("application/json")),
            ),
        ]))
    }
}

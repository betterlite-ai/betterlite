use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_core::api::search_providers::registry::SearchRegistry;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::search::{SearchServices, asearch};
use betterlite_cost::search::error::SearchCostError;
use betterlite_cost::search::search_provider_cost_per_query;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::error::plain_error;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;

use self::logging::{class_failure, runtime_config};
use super::convert::{
    ConvertError, CostAnswer, CostCall, SearchAnswer, SearchCall, SessionWire, parse,
};
use super::error::{NativeFailure, SessionError};
use super::pricing::CatalogPricing;

mod logging;
mod ocr;

const POOL_CAPACITY: usize = 4;

// Everything one Python session owns: the environment snapshot, the TLS policy and the HTTP clients every call goes through.
pub(crate) struct SessionCore {
    environment: BTreeMap<String, String>,
    tls: TlsPolicy,
    proxy: Arc<ProxyPolicy>,
    core: CoreSettings,
    pool: ClientPool,
    redactor: Redactor,
    catalog: Arc<ModelCatalog>,
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
        let core = CoreSettings::load(&config, &environment)
            .map_err(|source| SessionError::Settings { source })?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| SessionError::Redactor { source })?;
        Ok(Self {
            environment,
            tls,
            proxy: Arc::new(proxy),
            core,
            pool: ClientPool::new(POOL_CAPACITY, TransportLimits::DEFAULT),
            redactor,
            catalog: BundledCatalog::default().get(),
        })
    }

    // litellm.search: the provider's config, one logging state and the answer plus LiteLLM's hidden params
    pub(crate) async fn search(&self, call: SearchCall) -> Result<String, NativeFailure> {
        let request = call.into_request()?;
        let registry = SearchRegistry {
            environment: &self.environment,
        };
        let sink = SilentSink;
        let env = self.pre_post_env(&sink);
        let mut state = self.logging_state(&request.search_provider)?;
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        let services = SearchServices {
            lookup: &registry,
            pool: &self.pool,
            tls: &self.tls,
            proxy: &self.proxy,
            core: &self.core,
            network_mock: false,
        };
        match asearch(&request, &services, &mut log).await {
            Ok(response) => encode(&SearchAnswer {
                response: &response,
                hidden_params: &response.hidden_params,
            }),
            Err(failure) => {
                let message = failure.to_string();
                Err(NativeFailure::Class {
                    class: failure.class,
                    message,
                    headers: failure.response_headers,
                })
            }
        }
    }

    // litellm.search.cost: the cost map answers, and a model it does not hold becomes ModelNotMappedError in Python
    pub(crate) async fn cost(&self, call: CostCall) -> Result<String, NativeFailure> {
        let pricing = CatalogPricing {
            catalog: Arc::clone(&self.catalog),
        };
        let negative = call.number_of_queries < 0;
        let queries = call.number_of_queries.unsigned_abs();
        let outcome = search_provider_cost_per_query(
            &pricing,
            &call.model,
            call.custom_llm_provider.as_deref(),
            queries,
            &call.optional_params,
        )
        .await;
        let (input, output) = match outcome {
            Ok((input, output)) if negative => (-input, -output),
            Ok((input, output)) => (input, output),
            Err(SearchCostError::ModelInfo { source, .. }) => {
                return encode(&CostAnswer::Unmapped {
                    message: source.to_string(),
                });
            }
            Err(SearchCostError::Overflow) => {
                return Err(class_failure(
                    plain_error("the search cost arithmetic overflowed a decimal"),
                    Vec::new(),
                ));
            }
        };
        let (Some(input_cost), Some(output_cost)) = (input.to_f64(), output.to_f64()) else {
            return Err(NativeFailure::Convert(ConvertError::CostRange));
        };
        encode(&CostAnswer::Cost {
            input_cost,
            output_cost,
        })
    }
}

fn encode<T>(answer: &T) -> Result<String, NativeFailure>
where
    T: Serialize,
{
    serde_json::to_string(answer)
        .map_err(|source| NativeFailure::Convert(ConvertError::Encode { source }))
}

// Search logging writes records nobody subscribed to; the Python call has no callbacks to hand them on.
struct SilentSink;

impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}

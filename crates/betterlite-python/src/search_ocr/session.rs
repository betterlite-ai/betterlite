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
use betterlite_ocr::ocr_input::wire::{OcrWireRequest, decode_request_input};
use betterlite_ocr::ocr_route::aocr;
use betterlite_ocr::ocr_settings::OcrSettings;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::error::plain_error;
use serde::Serialize;

use self::logging::{class_failure, logging_state, pre_post_env, provider_of};
use super::convert::{
    ConvertError, CostAnswer, CostCall, OcrAnswer, OcrCall, SearchAnswer, SearchCall, SessionWire,
    resolve_document,
};
use super::error::{NativeFailure, SessionError};
use super::pricing::CatalogPricing;
use crate::completion::setup::runtime_config;
use crate::search_ocr::ocr_client;

mod logging;
mod ocr;
mod tests;

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
        let wire: SessionWire =
            serde_json::from_str(text).map_err(|source| ConvertError::Json { source })?;
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
        let env = pre_post_env(self, &sink);
        let mut state = logging_state(self, &request.search_provider)?;
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
        encode(&cost_answer(&pricing, call).await?)
    }

    // litellm.ocr: bind the request, run the provider call and hand back the normalized response
    pub(crate) async fn ocr(
        &self,
        call: OcrCall,
        blobs: Vec<Vec<u8>>,
    ) -> Result<String, NativeFailure> {
        let provider = provider_of(&call.model, call.custom_llm_provider.as_deref()).to_owned();
        let model = call.model.clone();
        let document = resolve_document(&call.document, &blobs)
            .map_err(|error| ocr::ocr_failure(self, &error, &model, &provider))?;
        let optional_params = ocr::consumed_params(self, &call, &model, &provider)?;
        let wire = OcrWireRequest {
            model: call.model,
            document,
            api_key: call.api_key,
            api_base: call.api_base,
            custom_llm_provider: call.custom_llm_provider,
            extra_headers: call.extra_headers,
            optional_params,
            input_sources: call.input_sources,
            timeout_seconds: call.timeout_seconds,
        };
        let request = decode_request_input(wire)
            .map_err(|error| ocr::ocr_failure(self, &error, &model, &provider))?;
        let client = ocr_client::build(
            &self.pool,
            &self.tls,
            &self.proxy,
            OcrSettings::from_environment(&self.environment),
            &self.environment,
        )
        .map_err(|source| NativeFailure::Session(SessionError::Client { source }))?;
        match aocr(client, request).await {
            Ok(response) => encode(&OcrAnswer {
                response: &response,
                provider_native_response: response.provider_native_response.as_ref(),
            }),
            Err(error) => Err(ocr::ocr_failure(self, &error, &model, &provider)),
        }
    }
}

// the tiers charge per query, and a negative count keeps Python's sign on the pair
async fn cost_answer(
    pricing: &CatalogPricing,
    call: CostCall,
) -> Result<CostAnswer, NativeFailure> {
    let outcome = search_provider_cost_per_query(
        pricing,
        &call.model,
        call.custom_llm_provider.as_deref(),
        call.number_of_queries.unsigned_abs(),
        &call.optional_params,
    )
    .await;
    match outcome {
        Ok((input, output)) if call.number_of_queries.is_negative() => Ok(CostAnswer::Cost {
            input_cost: -input,
            output_cost: -output,
        }),
        Ok((input, output)) => Ok(CostAnswer::Cost {
            input_cost: input,
            output_cost: output,
        }),
        Err(SearchCostError::ModelInfo { source, .. }) => Ok(CostAnswer::Unmapped {
            message: source.to_string(),
        }),
        Err(SearchCostError::Overflow) => Err(class_failure(
            plain_error("the search cost arithmetic overflowed a decimal"),
            Vec::new(),
        )),
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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, GeneralSettings, LiteLlmSettings, ProxyTrust};
use betterlite_context::core_settings::CoreSettings;
use betterlite_core::api::search_providers::registry::SearchRegistry;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_core::search::{SearchServices, asearch};
use betterlite_cost::search::error::SearchCostError;
use betterlite_cost::search::search_provider_cost_per_query;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_ocr::json::Object as OcrObject;
use betterlite_ocr::ocr_error::{OcrError, OcrErrorKind, OcrTransportError};
use betterlite_ocr::ocr_input::arguments::consumed_optional_param_names;
use betterlite_ocr::ocr_input::wire::{OcrWireRequest, decode_request_input};
use betterlite_ocr::ocr_route::aocr;
use betterlite_ocr::ocr_settings::OcrSettings;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_wrapper::error::plain_error;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use chrono::Utc;
use indexmap::IndexMap;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;
use uuid::Uuid;

use super::convert::{
    ConvertError, CostAnswer, CostCall, OcrAnswer, OcrCall, SearchAnswer, SearchCall, SessionWire,
    parse, resolve_document,
};
use super::error::{NativeFailure, SessionError};
use super::ocr_client;
use super::pricing::CatalogPricing;

const POOL_CAPACITY: usize = 4;

// Everything one Python session owns: the environment snapshot, the TLS policy and the HTTP clients every call goes through.
pub(crate) struct SessionCore {
    environment: BTreeMap<String, String>,
    config: Config,
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
            config,
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

    // litellm.ocr: bind the request, run the provider call and hand back the normalized response
    pub(crate) async fn ocr(
        &self,
        call: OcrCall,
        blobs: &[Vec<u8>],
    ) -> Result<String, NativeFailure> {
        let provider = provider_of(&call.model, call.custom_llm_provider.as_deref()).to_owned();
        let model = call.model.clone();
        let document = resolve_document(&call.document, blobs)
            .map_err(|error| self.ocr_failure(&error, &model, &provider))?;
        let optional_params = self.consumed_params(&call, &model, &provider)?;
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
            .map_err(|error| self.ocr_failure(&error, &model, &provider))?;
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
            Err(error) => Err(self.ocr_failure(&error, &model, &provider)),
        }
    }

    // project_optional_fields: only the names the provider consumes reach the request
    fn consumed_params(
        &self,
        call: &OcrCall,
        model: &str,
        provider: &str,
    ) -> Result<OcrObject, NativeFailure> {
        let names = consumed_optional_param_names(model, Some(provider))
            .map_err(|error| self.ocr_failure(&error, model, provider))?;
        let mut kept = OcrObject::new();
        for (name, value) in &call.optional_params {
            if names.contains(&name.as_str()) {
                let _previous = kept.insert(name.clone(), value.clone());
            }
        }
        Ok(kept)
    }

    // litellm.exception_type over the OCR failure: the provider's status and body, or the request's own 400
    fn ocr_failure(&self, error: &OcrError, model: &str, provider: &str) -> NativeFailure {
        let prefix = format!("{provider}/");
        let unprefixed = model.strip_prefix(&prefix).unwrap_or(model);
        let is_request = matches!(error.kind(), OcrErrorKind::Request);
        let mut raw = RawException {
            model: unprefixed.to_owned(),
            custom_llm_provider: provider.to_owned(),
            error_str: error.to_string(),
            status_code: error.http_status_code(),
            status_code_is_synthesized: false,
            message: None,
            body: None,
            exception_type: if is_request {
                "ValueError".to_owned()
            } else {
                String::new()
            },
        };
        let mut headers = Vec::new();
        if let OcrError::Provider {
            headers: response_headers,
            body,
            ..
        } = error
        {
            raw.message = Some(body.clone());
            headers.clone_from(response_headers);
        } else if let OcrError::Transport(OcrTransportError::Http { body, .. }) = error {
            raw.message = Some(body.clone());
        }
        let class = dispatch(&DispatchInput::Raw(Box::new(raw)), &self.redactor);
        NativeFailure::Class {
            class,
            message: error.to_string(),
            headers,
        }
    }

    fn logging_state(&self, provider: &str) -> Result<LoggingState, NativeFailure> {
        let init = LoggingInit {
            model: format!("{provider}/search"),
            call_type: "search".to_owned(),
            start_time: Utc::now().fixed_offset(),
            litellm_call_id: Uuid::new_v4().to_string(),
            function_id: String::new(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: None,
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        };
        let mut context = CorrelationContext::default();
        let setup = CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        };
        LoggingState::new(
            init,
            setup,
            &self.redactor,
            || Uuid::new_v4().to_string(),
            || Duration::ZERO,
        )
        .map_err(|error| class_failure(plain_error(&error.to_string()), Vec::new()))
    }

    fn pre_post_env<'env>(&'env self, sink: &'env dyn LogSink) -> PrePostEnv<'env> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: true,
                key_info: true,
            },
            redactor: &self.redactor,
            log: sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: Utc::now().fixed_offset(),
        }
    }
}

// The provider _context names: custom_llm_provider, else the model's own prefix before the first slash
fn provider_of<'model>(
    model: &'model str,
    custom_llm_provider: Option<&'model str>,
) -> &'model str {
    match custom_llm_provider {
        Some(provider) => provider,
        None => model.split_once('/').map_or("", |(prefix, _rest)| prefix),
    }
}

fn class_failure(class: ErrorClass, headers: Vec<(String, String)>) -> NativeFailure {
    let message = class_message(&class);
    NativeFailure::Class {
        class,
        message,
        headers,
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

// A Python session has no config file, so every section takes its own default.
fn runtime_config() -> Config {
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

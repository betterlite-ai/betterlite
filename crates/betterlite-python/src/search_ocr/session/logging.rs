use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::types::{Config, GeneralSettings, LiteLlmSettings, ProxyTrust};
use betterlite_error::error::{ErrorClass, class_message};
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, LogSink, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_sdk::call_wrapper::error::plain_error;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings};
use chrono::Utc;
use indexmap::IndexMap;
use uuid::Uuid;

use super::SessionCore;
use crate::search_ocr::error::NativeFailure;

impl SessionCore {
    // litellm.search's logging object: the call's own ids, no messages and no callbacks to hand records on
    pub(super) fn logging_state(&self, provider: &str) -> Result<LoggingState, NativeFailure> {
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

    pub(super) fn pre_post_env<'env>(&'env self, sink: &'env dyn LogSink) -> PrePostEnv<'env> {
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
pub(super) fn provider_of<'model>(
    model: &'model str,
    custom_llm_provider: Option<&'model str>,
) -> &'model str {
    match custom_llm_provider {
        Some(provider) => provider,
        None => model.split_once('/').map_or("", |(prefix, _rest)| prefix),
    }
}

pub(super) fn class_failure(class: ErrorClass, headers: Vec<(String, String)>) -> NativeFailure {
    let message = class_message(&class);
    NativeFailure::Class {
        class,
        message,
        headers,
    }
}

// A Python session has no config file, so every section takes its own default.
pub(super) fn runtime_config() -> Config {
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

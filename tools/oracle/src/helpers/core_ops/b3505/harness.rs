use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::interactions::base::Logging;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;

use crate::helpers::HelperError;

use super::failed;

pub(super) struct Harness {
    pub(super) clients: HttpClients,
    pub(super) state: LoggingState,
    pub(super) record: Logging,
    pub(super) redactor: Redactor,
}

impl Harness {
    pub(super) fn new(litellm_call_id: &str) -> Result<Self, Box<HelperError>> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failed(&error))?;
        let state = logging_state(litellm_call_id, &redactor)?;
        let environment: BTreeMap<String, String> = BTreeMap::new();
        let proxy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .map_err(|error| failed(&error))?;
        let clients = HttpClients::new(
            TlsPolicy {
                verification: Verification::Enabled,
                ca_file: None,
                certificate: None,
                http2: false,
                ecdh_curve: None,
                user_agent: "betterlite-oracle/0".to_owned(),
            },
            Arc::new(proxy),
        );
        Ok(Self {
            clients,
            state,
            record: Logging::default(),
            redactor,
        })
    }
}

fn logging_state(
    litellm_call_id: &str,
    redactor: &Redactor,
) -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    let start_time = DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
        .map_err(|error| failed(&error))?;
    LoggingState::new(
        LoggingInit {
            model: String::new(),
            call_type: String::new(),
            start_time,
            litellm_call_id: litellm_call_id.to_owned(),
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
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(|error| failed(&error))
}

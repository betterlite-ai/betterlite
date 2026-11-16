use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use lru::LruCache;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};

use crate::agent_365_tokens::cache::{CachedOboToken, OBO_CACHE_CAPACITY, TOKEN_EXPIRY_MARGIN};
use crate::agent_365_tokens::error::{Agent365Error, Agent365TokenError};
use crate::agent_365_tokens::exchange::{
    FORM_CONTENT_TYPE, TOKEN_ENDPOINT_TEMPLATE, parse_token_reply, token_form,
};
use crate::agent_365_tokens::poster::{Agent365Poster, PostReply};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

pub mod cache;
pub mod error;
pub mod exchange;
pub mod payload;
pub mod poster;
pub mod wire;

mod tests;

pub const EVALUATE_URL: &str =
    "https://agent365.svc.cloud.microsoft/agents/tool-evaluation/evaluate";
pub const DEFENDER_STATUS_EVALUATED: &str = "Evaluated";
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 10.0;
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 1] = [GuardrailEventHooks::PreMcpCall];

// LiteLLM's CustomGuardrail subclass sets records_own_guardrail_information and nothing else on its switches
const AGENT_365_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: true,
    uses_apply_guardrail_interface: false,
    deny_text_on_error: false,
};

#[derive(Debug, Clone)]
pub struct Agent365Settings {
    pub guardrail_name: String,
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub request_timeout: Option<f64>,
    pub unreachable_fallback: UnreachableFallback,
    pub event_hook: Option<LitellmParamsMode>,
    pub default_on: bool,
}

pub struct Agent365Guardrail {
    pub base: GuardrailBase,
    pub settings: Agent365Settings,
    pub poster: Arc<dyn Agent365Poster>,
    pub request_timeout: Duration,
    pub cache: Mutex<LruCache<String, CachedOboToken>>,
    pub clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
}

impl Agent365Guardrail {
    pub fn new(
        settings: Agent365Settings,
        poster: Arc<dyn Agent365Poster>,
        strict_modes: bool,
        clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    ) -> Result<Self, Agent365Error> {
        // LiteLLM's constructor default is 10.0 seconds when the config carries no timeout
        let seconds = settings.request_timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        let request_timeout = Duration::try_from_secs_f64(seconds)
            .map_err(|source| Agent365Error::Timeout { seconds, source })?;
        let base = GuardrailBase::new(base_settings(&settings), strict_modes)?;
        let name = &settings.guardrail_name;
        tracing::info!("Initialized Microsoft Agent 365 guardrail: {name}");
        Ok(Self {
            base,
            settings,
            poster,
            request_timeout,
            cache: Mutex::new(LruCache::new(OBO_CACHE_CAPACITY)),
            clock,
        })
    }

    pub async fn get_obo_token(&self, assertion: &str) -> Result<SecretString, Agent365TokenError> {
        let cache_key = hex::encode(Sha256::digest(assertion.as_bytes()));
        let threshold = (self.clock)().checked_add(TOKEN_EXPIRY_MARGIN);
        let fresh = {
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            let cached = cache
                .peek(&cache_key)
                .filter(|entry| threshold.is_some_and(|limit| entry.expires_at > limit))
                .map(|entry| entry.access_token.clone());
            if cached.is_some() {
                let _promoted = cache.promote(&cache_key);
            }
            cached
        };
        if let Some(token) = fresh {
            return Ok(token);
        }
        let url = TOKEN_ENDPOINT_TEMPLATE.replace("{tenant_id}", &self.settings.tenant_id);
        let headers =
            HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static(FORM_CONTENT_TYPE))]);
        let form = token_form(&self.settings, assertion);
        let reply = self
            .post_allowing_error_status(&url, &headers, form.as_bytes())
            .await?;
        let grant = parse_token_reply(&reply)?;
        let Some(expires_at) = (self.clock)().checked_add(grant.expires_in) else {
            return Err(Agent365TokenError::TokenExpiry);
        };
        let entry = CachedOboToken {
            access_token: grant.access_token.clone(),
            expires_at,
        };
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let _evicted = cache.push(cache_key, entry);
        Ok(grant.access_token)
    }

    pub async fn post_allowing_error_status(
        &self,
        url: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<PostReply, Agent365TokenError> {
        self.poster
            .post(url, headers, body, self.request_timeout)
            .await
            .map_err(Agent365TokenError::Poster)
    }

    pub fn evict_obo_token(&self, assertion: &str) {
        let cache_key = hex::encode(Sha256::digest(assertion.as_bytes()));
        let _removed = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop(&cache_key);
    }
}

fn base_settings(settings: &Agent365Settings) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(settings.guardrail_name.clone()),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: settings.event_hook.clone(),
        default_on: settings.default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        // LiteLLM constructs this guardrail with run_in_parallel=True
        run_in_parallel: true,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: AGENT_365_SWITCHES,
    }
}

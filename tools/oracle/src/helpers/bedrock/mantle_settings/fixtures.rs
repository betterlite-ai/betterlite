use std::future::{Future, ready};
use std::panic::panic_any;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_bedrock::aws_settings::credential_cache::CredentialCache;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_context::local_ttl_cache::dual::{CacheEvent, Diagnostics, DualLocalCache};
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use serde_json::Value;

use super::input::{Keys, Reasoning};
use betterlite_oracle_helper_api::error::HelperError;

pub(super) fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: "MantleSettingsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn credentials(keys: &Keys) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(keys.access_key_id.clone()),
        secret_access_key: SecretString::from(keys.secret_access_key.clone()),
        session_token: keys.session_token.clone().map(SecretString::from),
    }
}

pub(super) fn credential_cache() -> Result<Arc<CredentialCache>, Box<HelperError>> {
    let memory = LocalTtlCache::with_clock(Some(1000), None, None, 1024, Arc::new(|| 0.0_f64))
        .map_err(failure)?;
    let emit = Arc::new(|_event: CacheEvent| {});
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(16).map_err(failure)?),
        emit,
    };
    let cache = CredentialCache::new(DualLocalCache {
        memory: Arc::new(Mutex::new(memory)),
        default_in_memory_ttl: None,
        diagnostics,
    })
    .map_err(failure)?;
    Ok(Arc::new(cache))
}

#[derive(Clone, Default)]
pub(super) struct Seen(Arc<Mutex<Vec<(String, Option<String>)>>>);

impl Seen {
    pub(super) fn take(&self) -> Vec<Value> {
        let seen = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        seen.into_iter()
            .map(|(model, provider)| serde_json::json!([model, provider]))
            .collect()
    }
}

pub(super) struct Policy {
    pub(super) answer: Reasoning,
    pub(super) seen: Seen,
}

impl Policy {
    pub(super) fn new(answer: Reasoning) -> Self {
        Self {
            answer,
            seen: Seen::default(),
        }
    }
}

impl CapabilityPredicates for Policy {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.seen
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                request.model.to_owned(),
                request.custom_llm_provider.map(str::to_owned),
            ));
        let answer = self.answer;
        let is_reasoning = key == "supports_reasoning";
        async move {
            match answer {
                Reasoning::Supported => is_reasoning,
                Reasoning::Unsupported => false,
                Reasoning::Panic => panic_any("fixture reasoning probe"),
            }
        }
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

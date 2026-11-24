use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::batch_limit_descriptors::Settings;
use betterlite_config::sections::rate_limiter_settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_limit_descriptors::BatchLimitDescriptors;
use betterlite_core::api::batch_limit_descriptors::input::{
    BatchLimitCall, BatchLimitDescriptorsInput, BatchLimitPrincipal,
};
use betterlite_core::api::model_credentials::CredentialSource;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;
use tracing::subscriber;
use tracing_subscriber::layer::SubscriberExt as _;

mod capture;
mod rates;

use rates::RateLimitSource;

// serde_with's double_option needs Option<Option<T>>; the alias keeps clippy::option_option quiet
type OptionalScript = Option<Script>;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Script {
    models: BTreeMap<String, Option<JsonObject>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: Settings,
    principal: BatchLimitPrincipal,
    // a missing router is LiteLLM's llm_router None, a null one is a router that answered None
    #[serde(default, with = "serde_with::rust::double_option")]
    router: Option<OptionalScript>,
    calls: Vec<BatchLimitCall>,
}

struct ScriptedRouter {
    script: Option<Script>,
    asked: Mutex<Vec<String>>,
}

impl ScriptedRouter {
    fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CredentialSource for ScriptedRouter {
    type Error = Infallible;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        _team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, Infallible> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_id.to_owned());
        let Some(script) = &self.script else {
            return Ok(None);
        };
        Ok(script.models.get(model_id).cloned().flatten())
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BatchLimitDescriptorsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let window_size = rate_limiter_settings::Settings::default().window_size;
        let events = Arc::new(Mutex::new(Vec::new()));
        let credentials = input.router.clone().map(|script| ScriptedRouter {
            script,
            asked: Mutex::new(Vec::new()),
        });
        let service = BatchLimitDescriptors {
            settings: input.settings.clone(),
            limiter: RateLimitSource { window_size },
            credentials,
            limits: ApiLimits::default(),
            warned_unsupported_model_skip: AtomicBool::new(false),
        };
        let record = BatchLimitDescriptorsInput {
            principal: input.principal.clone(),
            calls: input.calls.clone(),
        };
        let guard = subscriber::set_default(
            tracing_subscriber::registry().with(capture::Capture(Arc::clone(&events))),
        );
        let outcome = service.run(&record);
        drop(guard);
        let result = outcome.map_err(|error| failure(error.to_string()))?;
        let answers =
            serde_json::to_value(&result.answers).map_err(|error| failure(error.to_string()))?;
        let asked = service
            .credentials
            .as_ref()
            .map_or_else(Vec::new, ScriptedRouter::asked);
        let captured = events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({
            "answers": answers,
            "events": captured,
            "asked": asked,
        }))
    }
}

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_roi::estimator::cache::cache_context;
use betterlite_roi::estimator::evidence::metadata_evidence;
use betterlite_roi::estimator::extraction::{first_json_object, mask_value_constants};
use betterlite_roi::estimator::models::{EstimatorModel, ROICompletionRequest};
use betterlite_roi::estimator::options::{configured_models, estimator_options};
use betterlite_roi::estimator::result::ROIEstimatorResult;
use betterlite_roi::records::ROIPullEvidence;
use betterlite_roi::settings::ROISettings;
use betterlite_server::roi_estimator::{CompletionCaller, CompletionFuture, Estimator};
use betterlite_server::roi_github::error::SourceError;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Input {
    pub operation: String,
    #[serde(default)]
    pub settings: Option<SettingsInput>,
    #[serde(default)]
    pub models: Option<Vec<EstimatorModel>>,
    #[serde(default)]
    pub supported: Vec<String>,
    #[serde(default)]
    pub pull: Option<ROIPullEvidence>,
    #[serde(default)]
    pub response: Option<Value>,
    #[serde(default)]
    pub response_is_error: bool,
    #[serde(default)]
    pub raw: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SettingsInput {
    pub estimator_model: String,
    pub estimator_prompt: String,
}

struct ModelSupport {
    supported: Vec<String>,
    seen: Vec<(String, String)>,
}

impl CapabilityPredicates for ModelSupport {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.seen.push((key.to_owned(), request.model.to_owned()));
        let supported = key == NamedCapability::NoneReasoningEffort.key()
            && self.supported.iter().any(|model| model == request.model);
        ready(supported)
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

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        block_on(run(input))?
    }
}

fn block_on<F>(future: F) -> Result<F::Output, Box<HelperError>>
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| runtime.block_on(future))
        .map_err(|error| helper_error("ValueError", error.to_string()))
}

async fn run(input: &Input) -> HelperResult {
    match input.operation.as_str() {
        "metadata_evidence" => serde_json::to_value(metadata_evidence(pull_of(input)?))
            .map_err(|error| helper_error("ValueError", error.to_string())),
        "estimator_options" => {
            let Some(models) = input.models.as_ref() else {
                return Err(helper_error(
                    "ValueError",
                    "estimator_options needs models".to_owned(),
                ));
            };
            let options = estimator_options(&mut support_of(input), models).await;
            serde_json::to_value(options)
                .map_err(|error| helper_error("ValueError", error.to_string()))
        }
        "configured_models" => models(input),
        "cache_context" => context(input).await,
        "estimate" => estimate(input).await,
        "validate_result" => validate(input),
        other => Err(helper_error(
            "ValueError",
            format!("unknown b3154 operation: {other}"),
        )),
    }
}

fn pull_of(input: &Input) -> Result<&ROIPullEvidence, Box<HelperError>> {
    input
        .pull
        .as_ref()
        .ok_or_else(|| helper_error("ValueError", "the operation needs a pull".to_owned()))
}

fn settings_of(input: &Input) -> Result<ROISettings, Box<HelperError>> {
    let settings = input
        .settings
        .as_ref()
        .ok_or_else(|| helper_error("ValueError", "the operation needs settings".to_owned()))?;
    Ok(ROISettings {
        github_api_url: "https://api.github.com".to_owned(),
        github_token: SecretString::default(),
        estimator_key: SecretString::default(),
        repos: Vec::new(),
        estimator_model: settings.estimator_model.clone(),
        estimator_prompt: settings.estimator_prompt.clone(),
        backfill_days: 7,
        update_interval_minutes: 1440.0_f64,
        identity_map: IndexMap::new(),
    })
}

fn support_of(input: &Input) -> ModelSupport {
    ModelSupport {
        supported: input.supported.clone(),
        seen: Vec::new(),
    }
}

fn models(input: &Input) -> HelperResult {
    let settings = settings_of(input)?;
    let pairs = configured_models(&settings, input.models.as_deref());
    serde_json::to_value(pairs).map_err(|error| helper_error("ValueError", error.to_string()))
}

async fn context(input: &Input) -> HelperResult {
    let settings = settings_of(input)?;
    let digest = cache_context(&settings, input.models.as_deref(), &mut support_of(input))
        .await
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(Value::String(digest))
}

async fn estimate(input: &Input) -> HelperResult {
    let settings = settings_of(input)?;
    let pull = pull_of(input)?.clone();
    let response = input.response.clone().unwrap_or(Value::Null);
    let raw_response =
        to_raw_value(&response).map_err(|error| helper_error("ValueError", error.to_string()))?;
    let response_is_error = input.response_is_error;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let complete: CompletionCaller = Arc::new(move |request: ROICompletionRequest| {
        if let Ok(mut recorded) = seen.lock() {
            recorded.push(request);
        }
        let reply = if response_is_error {
            Err(SourceError::Http {
                label: "estimator",
                status: 502,
            })
        } else {
            Ok(raw_response.clone())
        };
        let future: CompletionFuture = Box::pin(ready(reply));
        future
    });
    let estimator = Estimator::new(settings, complete, input.models.as_deref());
    let estimate = estimator
        .estimate(&pull, &mut support_of(input))
        .await
        .map_err(|error| helper_error("SourceError", error.to_string()))?;
    let recorded = requests
        .lock()
        .map_or_else(|_| Vec::new(), |guard| guard.clone());
    Ok(json!({"estimate": estimate, "requests": recorded}))
}

fn validate(input: &Input) -> HelperResult {
    let raw = input
        .raw
        .as_ref()
        .ok_or_else(|| helper_error("ValueError", "validate_result needs raw JSON".to_owned()))?;
    let object =
        first_json_object(raw).map_err(|error| helper_error("ValueError", error.to_string()))?;
    let masked = mask_value_constants(object);
    let result: ROIEstimatorResult = serde_json::from_str(&masked)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(json!({"hours": result.hours, "reasoning": result.reasoning}))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

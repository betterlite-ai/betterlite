use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::enkryptai_detection::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::enkryptai::process::process;
use betterlite_guardrails::enkryptai::wire::{DetectPayload, DetectionResponse};
use betterlite_guardrails::enkryptai::{
    API_PATH, EnkryptAiGuardrail, GUARDRAIL_PROVIDER, SUPPORTED_EVENT_HOOKS,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_http::proxy::base_builder;
use serde::Deserialize;
use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::{MockScript, ProviderCall};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Construct {
        settings: Settings,
    },
    Detect {
        settings: Settings,
        texts: Vec<String>,
        answer: UpstreamAnswer,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    api_key: Option<String>,
    api_base: Option<String>,
    policy_name: Option<String>,
    deployment_name: Option<String>,
    detectors: Option<BTreeMap<String, GuardrailValue>>,
    block_on_violation: Option<bool>,
}

struct DiscardSink;

impl GuardrailSpanSink for DiscardSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
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

fn configure(settings: &Settings) -> Result<Configured, Box<HelperError>> {
    let document = json!({
        "guardrails": [{
            "guardrail_name": "guardrail",
            "litellm_params": {
                "guardrail": "enkryptai",
                "api_key": settings.api_key,
                "api_base": settings.api_base,
                "policy_name": settings.policy_name,
                "deployment_name": settings.deployment_name,
                "detectors": settings.detectors,
                "block_on_violation": settings.block_on_violation,
            }
        }]
    });
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&document.to_string())
    .map_err(|error| config_failure(&error))?;
    config
        .sections
        .enkryptai_detection
        .guardrails
        .into_iter()
        .next()
        .ok_or_else(|| helper_error("ConfigError", "no EnkryptAI guardrail".to_owned()))
}

fn config_failure(error: &ConfigError) -> Box<HelperError> {
    let class = match error {
        ConfigError::MissingEnvironment { .. } => "EnkryptAIGuardrailMissingSecrets",
        ConfigError::Yaml { .. }
        | ConfigError::Unsupported { .. }
        | ConfigError::InvalidFloat { .. }
        | ConfigError::InvalidInteger { .. }
        | ConfigError::InvalidScalar { .. }
        | ConfigError::Environment { .. } => "ConfigError",
    };
    helper_error(class, error.to_string())
}

fn construct(settings: &Settings) -> HelperResult {
    let configured = configure(settings)?;
    Ok(json!({
        "api_base": configured.api_base,
        "api_url": format!("{}{}", configured.api_base, API_PATH),
        "policy_name": configured.policy_name,
        "provider": GUARDRAIL_PROVIDER,
        "hooks": SUPPORTED_EVENT_HOOKS,
    }))
}

struct Prepared {
    mock: MockProvider,
    guardrail: EnkryptAiGuardrail,
    runtime: tokio::runtime::Runtime,
}

struct DetectArgs<'args> {
    settings: &'args Settings,
    texts: Vec<String>,
    answer: &'args UpstreamAnswer,
}

fn prepare(args: &DetectArgs<'_>) -> Result<Prepared, Box<HelperError>> {
    let mut configured = configure(args.settings)?;
    let replies = vec![(*args.answer).clone(); args.texts.len().max(1)];
    let script = MockScript::try_from(replies.as_slice())
        .map_err(|error| helper_error("MockError", error.to_string()))?;
    let mock = MockProvider::start(script)
        .map_err(|error| helper_error("MockError", error.to_string()))?;
    configured.api_base.clone_from(&mock.base_url);
    let client = base_builder()
        .build()
        .map_err(|error| helper_error("OSError", error.to_string()))?;
    let guardrail = EnkryptAiGuardrail::new(
        configured,
        client,
        SystemTime::now,
        Arc::new(DiscardSink),
        true,
    )
    .map_err(|error| helper_error("ValueError", error.to_string()))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error("OSError", error.to_string()))?;
    Ok(Prepared {
        mock,
        guardrail,
        runtime,
    })
}

fn detect(args: DetectArgs<'_>) -> HelperResult {
    let prepared = prepare(&args)?;
    let Prepared {
        mock,
        guardrail,
        runtime,
    } = prepared;
    let inputs = GuardrailApiInputs {
        texts: Some(args.texts),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    let mut request_data = BTreeMap::from([(
        "model".to_owned(),
        GuardrailValue::Text("enkryptai".to_owned()),
    )]);
    let outcome = runtime.block_on(guardrail.apply_guardrail(
        inputs,
        &mut request_data,
        GuardrailInputType::Request,
        None,
    ));
    let calls = mock
        .finish()
        .map_err(|error| helper_error("MockError", error.to_string()))?;
    let processed = process_answer(args.answer)?;
    Ok(json!({
        "requests": request_views(&calls)?,
        "raised": outcome.is_err(),
        "statuses": statuses(&request_data),
        "processed": processed,
        "result": result_value(&outcome)?,
    }))
}

fn result_value(
    outcome: &Result<GuardrailApiInputs, GuardrailFailure>,
) -> Result<serde_json::Value, Box<HelperError>> {
    outcome.as_ref().map_or(Ok(json!(null)), |returned| {
        serde_json::to_value(returned)
            .map_err(|error| helper_error("ValidationError", error.to_string()))
    })
}

fn request_views(calls: &[ProviderCall]) -> Result<Vec<serde_json::Value>, Box<HelperError>> {
    let mut views = Vec::new();
    for call in calls {
        let body: DetectPayload = serde_json::from_value(call.request.body.clone())
            .map_err(|error| helper_error("ValidationError", error.to_string()))?;
        views.push(json!({
            "method": call.request.method,
            "path": call.request.path,
            "headers": {
                "content_type": call.request.headers.get("content-type"),
                "apikey": call.request.headers.get("apikey"),
                "x_enkrypt_policy": call.request.headers.get("x-enkrypt-policy"),
            },
            "body": body,
        }));
    }
    Ok(views)
}

fn process_answer(answer: &UpstreamAnswer) -> Result<serde_json::Value, Box<HelperError>> {
    let body = answer.body.clone().ok_or_else(|| {
        helper_error("ValidationError", "the answer needs a JSON body".to_owned())
    })?;
    let response: DetectionResponse = serde_json::from_value(body)
        .map_err(|error| helper_error("ValidationError", error.to_string()))?;
    let processed = process(&response);
    let all = serde_json::to_value(&response.details)
        .map_err(|error| helper_error("ValidationError", error.to_string()))?;
    let mut details = serde_json::Map::new();
    for attack in &processed.attacks {
        let detail = match all.get(attack) {
            None | Some(serde_json::Value::Null) => json!({}),
            Some(detail) => detail.clone(),
        };
        details.insert(attack.clone(), detail);
    }
    Ok(json!({"attacks": processed.attacks, "details": details}))
}

fn statuses(request_data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
    let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata") else {
        return Vec::new();
    };
    let Some(GuardrailValue::Array(records)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        return Vec::new();
    };
    records
        .iter()
        .filter_map(|record| {
            let GuardrailValue::Object(fields) = record else {
                return None;
            };
            let Some(GuardrailValue::Text(status)) = fields.get("guardrail_status") else {
                return None;
            };
            Some(status.clone())
        })
        .collect()
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Construct { settings } => construct(settings),
            Input::Detect {
                settings,
                texts,
                answer,
            } => detect(DetectArgs {
                settings,
                texts: texts.clone(),
                answer,
            }),
        }
    }
}

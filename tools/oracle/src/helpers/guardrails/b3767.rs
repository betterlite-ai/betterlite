use std::collections::BTreeMap;
use std::net::TcpListener;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::deepkeep_firewall::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::deepkeep::error::DeepKeepError;
use betterlite_guardrails::deepkeep::{
    DeepKeepGuardrail, InputType, Inputs, RequestContext, SUPPORTED_EVENT_HOOKS,
};
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Construct {
        settings: Value,
    },
    Metadata {
        request_data: BTreeMap<String, GuardrailValue>,
    },
    Apply {
        settings: Value,
        inputs: Box<Inputs>,
        #[serde(default)]
        request_data: BTreeMap<String, GuardrailValue>,
        #[serde(default)]
        dynamic_params: BTreeMap<String, GuardrailValue>,
        litellm_call_id: Option<String>,
        litellm_trace_id: Option<String>,
        #[serde(default)]
        input_type: InputType,
        answer: Option<UpstreamAnswer>,
    },
}

struct ApplyArgs<'input> {
    settings: &'input Value,
    inputs: Inputs,
    request_data: &'input BTreeMap<String, GuardrailValue>,
    dynamic_params: &'input BTreeMap<String, GuardrailValue>,
    litellm_call_id: Option<&'input str>,
    litellm_trace_id: Option<&'input str>,
    input_type: InputType,
    answer: Option<&'input UpstreamAnswer>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Construct { settings } => construct(settings),
            Input::Metadata { request_data } => Ok(json!(
                DeepKeepGuardrail::extract_user_api_key_metadata(request_data)
            )),
            Input::Apply {
                settings,
                inputs,
                request_data,
                dynamic_params,
                litellm_call_id,
                litellm_trace_id,
                input_type,
                answer,
            } => apply(ApplyArgs {
                settings,
                inputs: inputs.as_ref().clone(),
                request_data,
                dynamic_params,
                litellm_call_id: litellm_call_id.as_deref(),
                litellm_trace_id: litellm_trace_id.as_deref(),
                input_type: *input_type,
                answer: answer.as_ref(),
            }),
        }
    }
}

fn construct(settings: &Value) -> HelperResult {
    let configured = configure(settings, BTreeMap::new())?;
    Ok(json!({
        "api_base": configured.api_base,
        "timeout_ms": configured.timeout.as_millis(),
        "supported_event_hooks": SUPPORTED_EVENT_HOOKS,
    }))
}

type Prepared = (
    Option<MockProvider>,
    TcpListener,
    String,
    tokio::runtime::Runtime,
);

fn prepare(answer: Option<&UpstreamAnswer>) -> Result<Prepared, Box<HelperError>> {
    let fail = |class: &str, message: String| -> Box<HelperError> {
        Box::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        })
    };
    let io = |error: std::io::Error| fail("OSError", error.to_string());
    // with no answer this stays bound and silent until apply drops it, so the call times out instead of being refused
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    let mock = answer
        .map(|reply| {
            MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start)
        })
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io)?;
    Ok((mock, silent, base, runtime))
}

fn apply(args: ApplyArgs<'_>) -> HelperResult {
    let ApplyArgs {
        settings,
        inputs,
        request_data,
        dynamic_params,
        litellm_call_id,
        litellm_trace_id,
        input_type,
        answer,
    } = args;
    let (mock, silent, base, runtime) = prepare(answer)?;
    let mut deepkeep = guardrail(settings)?;
    deepkeep.settings.api_base = format!("{base}/v3/openai/beta/litellm_basic_guardrail_api");
    let context = RequestContext {
        request_data: request_data.clone(),
        input_type,
        litellm_call_id: litellm_call_id.map(ToOwned::to_owned),
        litellm_trace_id: litellm_trace_id.map(ToOwned::to_owned),
        litellm_version: "1.105.1".to_owned(),
        dynamic_params: dynamic_params.clone(),
    };
    let result = runtime.block_on(deepkeep.apply(inputs, context));
    drop(silent);
    finish(mock, result)
}

fn finish(mock: Option<MockProvider>, result: Result<Inputs, DeepKeepError>) -> HelperResult {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| {
            Box::new(HelperError {
                class: "MockError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let request = recorded.map(|call| call.request.body);
    Ok(json!({
        "path": path,
        "request": request,
        "result": result.map_err(|error| deepkeep_failure(&error))?,
    }))
}

fn configure(
    settings: &Value,
    lookup: BTreeMap<String, String>,
) -> Result<Configured, Box<HelperError>> {
    let mut parameters = settings.clone();
    if let Value::Object(map) = &mut parameters {
        map.insert("guardrail".to_owned(), json!("deepkeep"));
    }
    let entry = json!({"guardrail_name": "firewall", "litellm_params": parameters});
    let loaded = ConfigLoader { lookup }.load(&json!({"guardrails": [entry]}).to_string());
    let outcome = loaded.map(|config| {
        config
            .sections
            .deepkeep_firewall
            .guardrails
            .into_iter()
            .next()
    });
    let (class, message) = match outcome {
        Ok(Some(configured)) => return Ok(configured),
        Ok(None) => ("ConfigError", "no DeepKeep guardrail".to_owned()),
        Err(error) => {
            let class = match error {
                ConfigError::MissingEnvironment { .. } => "DeepKeepGuardrailMissingSecrets",
                ConfigError::Yaml { .. }
                | ConfigError::Unsupported { .. }
                | ConfigError::InvalidFloat { .. }
                | ConfigError::InvalidInteger { .. }
                | ConfigError::InvalidScalar { .. }
                | ConfigError::InvalidDuration { .. } => "ConfigError",
            };
            (class, error.to_string())
        }
    };
    Err(Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }))
}

fn guardrail(settings: &Value) -> Result<DeepKeepGuardrail, Box<HelperError>> {
    let configured = configure(settings, BTreeMap::new())?;
    let client = reqwest::Client::builder().build().map_err(|error| {
        Box::new(HelperError {
            class: "OSError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok(DeepKeepGuardrail {
        settings: configured,
        client,
    })
}

fn deepkeep_failure(error: &DeepKeepError) -> Box<HelperError> {
    let class = match error {
        DeepKeepError::Blocked { .. } => "GuardrailRaisedException",
        DeepKeepError::Api { .. }
        | DeepKeepError::ApiEncoding { .. }
        | DeepKeepError::ResponseJson { .. }
        | DeepKeepError::ResponseTooLarge { .. }
        | DeepKeepError::Request { .. }
        | DeepKeepError::RequestBuild { .. }
        | DeepKeepError::Timeout { .. }
        | DeepKeepError::InvalidHeader { .. }
        | DeepKeepError::InvalidHeaderName { .. } => "DeepKeepGuardrailAPIError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::deepkeep_firewall::Configured;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::deepkeep::{
    DeepKeepError, DeepKeepGuardrail, InputType, Inputs, RequestContext,
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
    Ok(json!({"api_base": configured.api_base}))
}

fn apply(args: ApplyArgs<'_>) -> HelperResult {
    let io = |error: std::io::Error| failure("OSError", error.to_string());
    let mock_error = |error: crate::OracleError| failure("MockError", error.to_string());
    // with no answer the listener accepts the connection and never replies, so the call times out
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
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    let start =
        |reply| MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start);
    let mock = answer.map(start).transpose().map_err(mock_error)?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };
    let mut deepkeep = guardrail(settings)?;
    deepkeep.settings.api_base = format!("{base}/v3/openai/beta/litellm_basic_guardrail_api");
    deepkeep.timeout = Duration::from_millis(200);
    let context = context(
        request_data,
        dynamic_params,
        litellm_call_id,
        litellm_trace_id,
        input_type,
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io)?;
    let result = runtime.block_on(deepkeep.apply(inputs, context));
    finish(mock, result)
}

fn context(
    request_data: &BTreeMap<String, GuardrailValue>,
    dynamic_params: &BTreeMap<String, GuardrailValue>,
    litellm_call_id: Option<&str>,
    litellm_trace_id: Option<&str>,
    input_type: InputType,
) -> RequestContext {
    let request_data = request_data.clone();
    let dynamic_params = dynamic_params.clone();
    let litellm_call_id = litellm_call_id.map(ToOwned::to_owned);
    let litellm_trace_id = litellm_trace_id.map(ToOwned::to_owned);
    RequestContext {
        request_data,
        input_type,
        litellm_call_id,
        litellm_trace_id,
        litellm_version: None,
        dynamic_params,
    }
}

fn finish(mock: Option<MockProvider>, result: Result<Inputs, DeepKeepError>) -> HelperResult {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| failure("MockError", error.to_string()))?;
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
    let config = loaded.map_err(|error| {
        let class = match error {
            ConfigError::MissingEnvironment { .. } => "DeepKeepGuardrailMissingSecrets",
            ConfigError::Yaml { .. }
            | ConfigError::Unsupported { .. }
            | ConfigError::InvalidFloat { .. }
            | ConfigError::InvalidInteger { .. }
            | ConfigError::InvalidScalar { .. } => "ConfigError",
        };
        failure(class, error.to_string())
    })?;
    let guardrails = config.sections.deepkeep_firewall.guardrails;
    guardrails
        .into_iter()
        .next()
        .ok_or_else(|| failure("ConfigError", "no DeepKeep guardrail".to_owned()))
}

fn guardrail(settings: &Value) -> Result<DeepKeepGuardrail, Box<HelperError>> {
    let configured = configure(settings, BTreeMap::new())?;
    let client = reqwest::Client::builder()
        .build()
        .map_err(|error| failure("OSError", error.to_string()))?;
    Ok(DeepKeepGuardrail {
        settings: configured,
        client,
        timeout: betterlite_guardrails::deepkeep::DEFAULT_TIMEOUT,
    })
}

fn deepkeep_failure(error: &DeepKeepError) -> Box<HelperError> {
    let class = match error {
        DeepKeepError::Blocked { .. } => "GuardrailRaisedException",
        DeepKeepError::Api { .. }
        | DeepKeepError::ResponseJson { .. }
        | DeepKeepError::Request { .. }
        | DeepKeepError::Timeout { .. }
        | DeepKeepError::InvalidHeader { .. }
        | DeepKeepError::InvalidHeaderName { .. } => "DeepKeepGuardrailAPIError",
    };
    failure(class, error.to_string())
}

fn failure(class_name: &str, message: String) -> Box<HelperError> {
    let class = class_name.to_owned();
    Box::new(HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

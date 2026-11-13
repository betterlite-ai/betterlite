use std::collections::BTreeMap;
use std::net::TcpListener;
use std::time::Duration;

use betterlite_guardrails::openai_moderation::error::ModerationError;
use betterlite_guardrails::openai_moderation::logging::{GuardrailLogRecord, GuardrailResponse};
use betterlite_guardrails::openai_moderation::{
    GlobalApiKeys, Inputs, ModerationModel, ModerationParams, OpenAiModerationGuardrail,
    RequestData, SUPPORTED_EVENT_HOOKS,
};
use betterlite_http::proxy::base_builder;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::{MockProvider, ProviderRequest};
use crate::routing_mock_scripts::{MockScript, ProviderCall, redact};

pub(crate) struct Op;

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    api_key: Option<String>,
    api_base: Option<String>,
    model: Option<ModerationModel>,
    streaming_end_of_stream_only: Option<bool>,
    streaming_sampling_rate: Option<u32>,
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlobalKeys {
    api_key: Option<String>,
    openai_key: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Construct {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    global_keys: GlobalKeys,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Apply {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    global_keys: GlobalKeys,
    inputs: Inputs,
    #[serde(default)]
    request_data: RequestData,
    answer: Option<UpstreamAnswer>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Construct(Construct),
    Apply(Box<Apply>),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Construct(construct) => {
                let moderation = build(
                    &construct.settings,
                    &construct.environment,
                    &construct.global_keys,
                )?;
                Ok(json!({
                    "api_key": moderation.api_key.expose_secret(),
                    "api_base": moderation.api_base,
                    "model": moderation.model,
                    "streaming_end_of_stream_only": moderation.streaming_end_of_stream_only,
                    "streaming_sampling_rate": moderation.streaming_sampling_rate,
                    "supported_event_hooks": SUPPORTED_EVENT_HOOKS,
                }))
            }
            Input::Apply(apply) => apply_guardrail(apply),
        }
    }
}

fn build(
    settings: &Settings,
    environment: &BTreeMap<String, String>,
    global_keys: &GlobalKeys,
) -> Result<OpenAiModerationGuardrail, Box<HelperError>> {
    let params = ModerationParams {
        guardrail_name: "moderation".to_owned(),
        api_key: settings.api_key.clone().map(SecretString::from),
        api_base: settings.api_base.clone(),
        model: settings.model,
        streaming_end_of_stream_only: settings.streaming_end_of_stream_only,
        streaming_sampling_rate: settings.streaming_sampling_rate,
        timeout: None,
    };
    let keys = GlobalApiKeys {
        api_key: global_keys.api_key.clone().map(SecretString::from),
        openai_key: global_keys.openai_key.clone().map(SecretString::from),
    };
    OpenAiModerationGuardrail::new(params, keys, environment, base_builder(), || Duration::ZERO)
        .map_err(|error| {
            Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
}

fn start_mock(
    answer: Option<&UpstreamAnswer>,
    silent: &TcpListener,
) -> Result<(Option<MockProvider>, String), Box<HelperError>> {
    let mock = answer
        .map(|reply| {
            MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start)
        })
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
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!(
            "http://{}",
            silent.local_addr().map_err(|error| {
                Box::new(HelperError {
                    class: "OSError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?
        ),
    };
    Ok((mock, base))
}

fn apply_guardrail(apply: &Apply) -> HelperResult {
    // with no answer the listener stays bound and silent until the call ends, so the request is never refused
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| {
        Box::new(HelperError {
            class: "OSError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: "OSError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    let (mock, base) = start_mock(apply.answer.as_ref(), &silent)?;
    let mut moderation = build(&apply.settings, &apply.environment, &apply.global_keys)?;
    moderation.api_base = base;
    let mut request_data = apply.request_data.clone();
    let outcome =
        runtime.block_on(moderation.apply_guardrail(apply.inputs.clone(), &mut request_data));
    drop(silent);
    summarize(apply, &moderation, &outcome, &request_data, mock)
}

fn summarize(
    apply: &Apply,
    moderation: &OpenAiModerationGuardrail,
    outcome: &Result<Inputs, ModerationError>,
    request_data: &RequestData,
    mock: Option<MockProvider>,
) -> HelperResult {
    let calls: Vec<ProviderCall> = mock
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
        })?
        .unwrap_or_default();
    let sent = calls
        .into_iter()
        .next()
        .map(|call| sent_request(&call.request, moderation));
    Ok(json!({
        "request": sent,
        "inputs_unchanged": outcome.as_ref().ok() == Some(&apply.inputs),
        "error": outcome.as_ref().err().map(error_summary),
        "stash_left": request_data.metadata.openai_moderation_response.is_some(),
        "records": request_data
            .metadata
            .guardrail_records
            .iter()
            .map(record_summary)
            .collect::<Vec<_>>(),
    }))
}

fn sent_request(request: &ProviderRequest, moderation: &OpenAiModerationGuardrail) -> Value {
    // the mock stores credential headers as a hash, so the expected header goes through the same hash
    let mut expected = BTreeMap::from([(
        "authorization".to_owned(),
        format!("Bearer {}", moderation.api_key.expose_secret()),
    )]);
    redact(&mut expected);
    json!({
        "path": request.path.rsplit('/').next(),
        "body": request.body,
        "authorization_ok": request.headers.get("authorization") == expected.get("authorization"),
        "content_type": request.headers.get("content-type"),
    })
}

fn error_summary(error: &ModerationError) -> Value {
    // LiteLLM's HTTP handler masks every 4xx and 5xx upstream answer, so it carries no detail
    let masked = matches!(error, ModerationError::Api { status, .. } if status.as_u16() >= 400);
    let detail = if masked { None } else { error.detail() };
    json!({"status": error.status_code().as_u16(), "detail": detail})
}

fn record_summary(record: &GuardrailLogRecord) -> Value {
    let response = match &record.guardrail_response {
        GuardrailResponse::Text(text) if text != "allow" => json!("<error>"),
        GuardrailResponse::Text(_) | GuardrailResponse::Moderation(_) => {
            json!(record.guardrail_response)
        }
    };
    let mut summary = json!({
        "guardrail_name": record.guardrail_name,
        "guardrail_response": response,
        "guardrail_status": record.guardrail_status,
    });
    if let (Value::Object(fields), Some(detail)) = (&mut summary, &record.tracing_detail) {
        fields.insert(
            "violation_categories".to_owned(),
            json!(detail.violation_categories),
        );
    }
    summary
}

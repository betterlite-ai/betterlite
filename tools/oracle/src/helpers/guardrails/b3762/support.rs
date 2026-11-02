use std::net::TcpListener;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::cisco_verdict_contracts::Configured;
use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_inspection::construct::CiscoInspectionGuardrail;
use betterlite_guardrails::cisco_verdict_enforcement::api_error::ApiErrorOutcome;
use betterlite_guardrails::cisco_verdict_enforcement::finalize::FinalizeOutcome;
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::HelperError;
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

pub(super) fn fail(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn json_to_value(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(text) => serde_json::from_str(text).unwrap_or(Value::Null),
        Json::Text(text) => Value::String(text.clone()),
        Json::List(items) => Value::Array(
            items
                .iter()
                .map(|raw| serde_json::from_str(raw.get()).unwrap_or(Value::Null))
                .collect(),
        ),
        Json::Object(fields) => {
            serde_json::from_str(&serde_json::to_string(fields).unwrap_or_default())
                .unwrap_or(Value::Null)
        }
    }
}

pub(super) fn configure(settings: &Value) -> Result<Configured, Box<HelperError>> {
    let mut parameters = settings.clone();
    if let Value::Object(map) = &mut parameters {
        map.insert("guardrail".to_owned(), json!("cisco_ai_defense"));
    }
    let entry = json!({"guardrail_name": "cisco-ai-defense", "litellm_params": parameters});
    let loaded = ConfigLoader {
        lookup: std::collections::BTreeMap::<String, String>::new(),
    }
    .load(&json!({"guardrails": [entry]}).to_string());
    let outcome = loaded.map(|config| {
        config
            .sections
            .cisco_verdict_contracts
            .guardrails
            .into_iter()
            .next()
    });
    let (class, message) = match outcome {
        Ok(Some(configured)) => return Ok(configured),
        Ok(None) => ("ConfigError", "no Cisco AI Defense guardrail".to_owned()),
        Err(error) => {
            let class = match error {
                ConfigError::MissingEnvironment { .. } => "CiscoAIDefenseGuardrailMissingSecrets",
                ConfigError::Yaml { .. }
                | ConfigError::Unsupported { .. }
                | ConfigError::InvalidFloat { .. }
                | ConfigError::InvalidInteger { .. }
                | ConfigError::InvalidScalar { .. } => "ConfigError",
            };
            (class, error.to_string())
        }
    };
    Err(fail(class, message))
}

pub(super) fn guardrail(settings: &Value) -> Result<CiscoInspectionGuardrail, Box<HelperError>> {
    let configured = configure(settings)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| fail("OSError", error.to_string()))?;
    Ok(CiscoInspectionGuardrail {
        settings: configured,
        client,
    })
}

pub(super) type Prepared = (
    Option<MockProvider>,
    TcpListener,
    String,
    tokio::runtime::Runtime,
);

pub(super) fn prepare(answer: Option<&UpstreamAnswer>) -> Result<Prepared, Box<HelperError>> {
    let io = |error: std::io::Error| fail("OSError", error.to_string());
    // with no answer this stays bound and silent until the caller drops it, so the call times out instead of being refused
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

pub(super) fn recorded_call(
    mock: Option<MockProvider>,
) -> Result<(Option<String>, Option<Value>), Box<HelperError>> {
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(|error| fail("MockError", error.to_string()))?;
    let recorded = calls.unwrap_or_default().into_iter().next();
    let path = recorded.as_ref().map(|call| call.request.path.clone());
    let body = recorded.map(|call| call.request.body);
    Ok((path, body))
}

pub(super) fn outcome_tag(outcome: &FinalizeOutcome) -> Value {
    match outcome {
        FinalizeOutcome::Allow => json!({"tag": "allow"}),
        FinalizeOutcome::Redacted(_) => json!({"tag": "redacted"}),
        FinalizeOutcome::Monitored => json!({"tag": "monitored"}),
        FinalizeOutcome::Blocked(_) => json!({"tag": "blocked"}),
        FinalizeOutcome::ApiError(ApiErrorOutcome::Unscanned) => json!({"tag": "unscanned"}),
        FinalizeOutcome::ApiError(ApiErrorOutcome::Blocked(detail)) => json!({
            "tag": "api_error_blocked",
            "detail": {
                "error": detail.error,
                "message": detail.message,
                "error_type": detail.error_type,
            },
        }),
    }
}

pub(super) fn applied_guardrails(request_data: &Fields) -> Vec<String> {
    let bucket_key = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Json::Object(bucket) = Json::field(request_data, bucket_key) else {
        return Vec::new();
    };
    match Json::field(&bucket, "applied_guardrails") {
        Json::List(items) => items
            .iter()
            .filter_map(|item| match Json::parse(item.get()) {
                Json::Text(name) => Some(name),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Vec::new()
        }
    }
}

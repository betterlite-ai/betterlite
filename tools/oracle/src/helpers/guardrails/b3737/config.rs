use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::ibm_detector_client::client::IbmDetectorClient;
use betterlite_guardrails::ibm_detector_client::config::IbmDetectorConfig;
use betterlite_guardrails::ibm_hooks::IbmGuardrail;
use betterlite_guardrails::ibm_hooks::config::EventHook;
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(crate) type Settings = BTreeMap<String, GuardrailValue>;

const GUARDRAIL_NAME: &str = "ibm-detector";
const MOCK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn guardrail(
    settings: &Settings,
    base_url: Option<&str>,
) -> Result<IbmGuardrail, Box<HelperError>> {
    let (config, event_hooks) = resolve_config(settings, base_url)?;
    let client = IbmDetectorClient::new(config)
        .map_err(|error| failure("IbmDetectorClientError", error.to_string()))?;
    Ok(IbmGuardrail {
        client,
        event_hooks,
    })
}

fn resolve_config(
    settings: &Settings,
    base_url: Option<&str>,
) -> Result<(IbmDetectorConfig, Option<Vec<EventHook>>), Box<HelperError>> {
    let mut parameters = settings.clone();
    let event_hooks = event_hooks(parameters.remove("mode"))?;
    if let Some(base) = base_url {
        parameters.insert("base_url".to_owned(), GuardrailValue::Text(base.to_owned()));
    }
    let Value::Object(mut litellm_params) = serde_json::to_value(&parameters)
        .map_err(|error| failure("ConfigError", error.to_string()))?
    else {
        return Err(failure(
            "ConfigError",
            "the IBM guardrail settings are not an object".to_owned(),
        ));
    };
    litellm_params.insert(
        "guardrail".to_owned(),
        Value::String("ibm_guardrails".to_owned()),
    );
    let entry = json!({"guardrail_name": GUARDRAIL_NAME, "litellm_params": litellm_params});
    let document = json!({"guardrails": [entry]}).to_string();
    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&document)
    .map_err(|error| failure("ConfigError", error.to_string()))?;
    let configured = loaded
        .sections
        .ibm_guardrails
        .guardrails
        .into_iter()
        .next()
        .ok_or_else(|| failure("ConfigError", "no IBM guardrail".to_owned()))?;
    let mut config = IbmDetectorConfig::try_from(configured)
        .map_err(|error| failure("ValueError", error.to_string()))?;
    config.timeout = Some(MOCK_TIMEOUT);
    Ok((config, event_hooks))
}

fn event_hooks(value: Option<GuardrailValue>) -> Result<Option<Vec<EventHook>>, Box<HelperError>> {
    match value {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(name)) => Ok(Some(vec![event_hook(&name)?])),
        Some(GuardrailValue::Array(items)) => {
            let hooks = items
                .iter()
                .map(|item| match item {
                    GuardrailValue::Text(name) => event_hook(name),
                    GuardrailValue::Null
                    | GuardrailValue::Boolean(_)
                    | GuardrailValue::SignedInteger(_)
                    | GuardrailValue::UnsignedInteger(_)
                    | GuardrailValue::Decimal(_)
                    | GuardrailValue::Array(_)
                    | GuardrailValue::Object(_) => Err(failure(
                        "ValueError",
                        "mode entries must be strings".to_owned(),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(hooks))
        }
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Object(_),
        ) => Err(failure(
            "ValueError",
            "mode must be a string or a list of strings".to_owned(),
        )),
    }
}

fn event_hook(name: &str) -> Result<EventHook, Box<HelperError>> {
    serde_json::from_value(Value::String(name.to_owned())).map_err(|error| {
        failure(
            "ValueError",
            format!("unknown IBM guardrail mode `{name}`: {error}"),
        )
    })
}

pub(super) fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

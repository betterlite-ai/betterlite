use std::collections::BTreeMap;
use std::slice;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::cisco_verdict_contracts::Configured;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_inspection::construct::CiscoInspectionGuardrail;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_runner::case::UpstreamAnswer;
use betterlite_oracle_runner::mock::MockProvider;
use betterlite_oracle_runner::routing_mock_scripts::MockScript;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

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

pub(super) struct Harness {
    provider: MockProvider,
    pub(super) runtime: Runtime,
}

impl Harness {
    pub(super) fn start(answer: &UpstreamAnswer) -> Result<Self, Box<HelperError>> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| fail("OSError", error.to_string()))?;
        let script = MockScript::try_from(slice::from_ref(answer))
            .map_err(|error| fail("MockError", error.to_string()))?;
        let provider =
            MockProvider::start(script).map_err(|error| fail("MockError", error.to_string()))?;
        Ok(Self { provider, runtime })
    }

    pub(super) fn guardrail(
        &self,
        settings: &Value,
    ) -> Result<CiscoInspectionGuardrail, Box<HelperError>> {
        CiscoInspectionGuardrail::new(configured(settings, &self.provider.base_url)?)
            .map_err(|error| fail("OSError", error.to_string()))
    }

    // The inspect request the guardrail sent, or two nulls when it never called the endpoint.
    pub(super) fn recorded(&self) -> Result<(Option<String>, Option<Value>), Box<HelperError>> {
        let calls = self
            .provider
            .finish_run()
            .map_err(|error| fail("MockError", error.to_string()))?;
        let recorded = calls.into_iter().next();
        let path = recorded.as_ref().map(|call| call.request.path.clone());
        let body = recorded.map(|call| call.request.body);
        Ok((path, body))
    }
}

pub(super) fn selection(settings: &Value) -> Result<GuardrailBase, Box<HelperError>> {
    let event_hook = settings
        .get("event_hook")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value::<LitellmParamsMode>(value.clone()))
        .transpose()
        .map_err(|error| fail("InvalidInput", error.to_string()))?;
    let default_on = matches!(settings.get("default_on"), Some(Value::Bool(true)));
    let guardrail_name = Some("cisco-ai-defense".to_owned());
    let configured = GuardrailSettings {
        guardrail_name,
        supported_event_hooks: None,
        event_hook,
        default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches::default(),
    };
    GuardrailBase::new(configured, true)
        .map_err(|error| fail("GuardrailBaseError", error.to_string()))
}

pub(super) fn applied_guardrails(request_data: &Fields) -> Vec<String> {
    let Ok(text) = serde_json::to_string(request_data) else {
        return Vec::new();
    };
    let Ok(serialized) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let bucket = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    let Some(Value::Array(names)) = serialized
        .get(bucket)
        .and_then(|section| section.get("applied_guardrails"))
    else {
        return Vec::new();
    };
    names
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn configured(settings: &Value, api_base: &str) -> Result<Configured, Box<HelperError>> {
    let mut parameters = settings.clone();
    let Value::Object(fields) = &mut parameters else {
        return Err(fail(
            "InvalidInput",
            "settings must be an object".to_owned(),
        ));
    };
    // the gate's event_hook and default_on come from `selection`; the config section has no such keys
    fields.remove("event_hook");
    fields.remove("default_on");
    fields.insert("guardrail".to_owned(), json!("cisco_ai_defense"));
    fields.insert("api_base".to_owned(), json!(api_base));
    let entry = json!({"guardrails": [{"guardrail_name": "cisco-ai-defense", "litellm_params": parameters}]});
    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&entry.to_string())
    .map_err(|error| {
        let class = match error {
            ConfigError::MissingEnvironment { .. } => "CiscoAIDefenseGuardrailMissingSecrets",
            ConfigError::Yaml { .. }
            | ConfigError::Unsupported { .. }
            | ConfigError::InvalidFloat { .. }
            | ConfigError::InvalidInteger { .. }
            | ConfigError::InvalidScalar { .. }
            | ConfigError::Environment { .. }
            | ConfigError::CustomDimensions(_)
            | ConfigError::LoadThread { .. } => "ConfigError",
        };
        fail(class, error.to_string())
    })?;
    loaded
        .sections
        .cisco_verdict_contracts
        .guardrails
        .into_iter()
        .next()
        .ok_or_else(|| fail("ConfigError", "no Cisco guardrail configured".to_owned()))
}

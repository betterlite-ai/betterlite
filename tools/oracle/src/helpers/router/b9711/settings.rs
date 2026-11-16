use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_router::router_runtime_settings::error::RouterSettingsError;
use betterlite_router::router_runtime_settings::target::RouterSettingsTarget;
use betterlite_router::router_runtime_settings::{RouterRuntimeSettings, SettingsView};
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
pub(crate) struct SettingsInput {
    initial: Initial,
    kwargs: Vec<(String, Value)>,
}

#[derive(Debug, Deserialize)]
struct Initial {
    #[serde(default = "default_routing_strategy")]
    routing_strategy: String,
    #[serde(default)]
    routing_strategy_args: Map<String, Value>,
    routing_groups: Option<Value>,
}

fn default_routing_strategy() -> String {
    "simple-shuffle".to_owned()
}

#[derive(Default)]
struct Recorder {
    calls: Vec<Value>,
}

type Step = Result<(), RouterSettingsError>;

impl Recorder {
    fn record(&mut self, step: &str, argument: Value) -> Step {
        self.calls.push(json!([step, argument]));
        Ok(())
    }
}

impl RouterSettingsTarget for Recorder {
    fn set_optional_pre_call_checks(&mut self, checks: &Value) -> Step {
        self.record("set_optional_pre_call_checks", checks.clone())
    }

    fn init_routing_strategy(&mut self, strategy: &Value, args: &Value) -> Step {
        let call = json!({"routing_strategy": strategy, "routing_strategy_args": args});
        self.record("routing_strategy_init", call)
    }

    fn apply_updated_strategy_args(&mut self) -> Step {
        self.record("apply_updated_routing_strategy_args", Value::Null)
    }

    fn apply_lar1(&mut self, args: Option<&Value>) -> Step {
        self.record("apply_lar1", args.cloned().unwrap_or(Value::Null))
    }

    // The Python recorder validates the input as a list or None, which is where a bad value fails
    fn init_routing_groups(&mut self, input: Option<&Value>) -> Step {
        if input.is_some_and(|groups| !groups.is_array()) {
            return Err(RouterSettingsError::Target {
                name: "init_routing_groups".to_owned(),
                message: "routing_groups must be a list or null".to_owned(),
            });
        }
        self.record("init_routing_groups", input.cloned().unwrap_or(Value::Null))
    }
}

// The values a Router built with no arguments stores for these names
fn seeded_settings(initial: &Initial) -> RouterRuntimeSettings {
    let values = json!({
        "routing_strategy": initial.routing_strategy,
        "routing_strategy_args": initial.routing_strategy_args,
        "allowed_fails": 3,
        "cooldown_time": 5,
        "num_retries": 2,
        "retry_after": 0,
        "retry_policy": null,
        "model_group_retry_policy": null,
    });
    RouterRuntimeSettings {
        values: values.as_object().cloned().unwrap_or_default(),
        routing_groups_input: initial.routing_groups.clone(),
    }
}

// Python names the class of the exception its own int() or pydantic raised
fn error_type(error: &RouterSettingsError, kwargs: &[(String, Value)]) -> &'static str {
    match error {
        RouterSettingsError::NotAnInteger { key } => {
            let refused = kwargs.iter().find(|(name, _)| name == key);
            if refused.is_some_and(|(_, value)| value.is_string()) {
                "ValueError"
            } else {
                "TypeError"
            }
        }
        RouterSettingsError::RetryPolicy { .. } | RouterSettingsError::Target { .. } => {
            "ValidationError"
        }
    }
}

pub(super) fn run(input: &SettingsInput) -> HelperResult {
    let mut settings = seeded_settings(&input.initial);
    let mut recorder = Recorder::default();
    let error = match settings.update_settings(&mut recorder, &input.kwargs) {
        Ok(()) => Value::Null,
        Err(error) => json!({
            "type": error_type(&error, &input.kwargs),
            "message": error.to_string(),
        }),
    };
    let shown = settings.get_settings(&SettingsView::default());
    let listed: Map<String, Value> = input
        .kwargs
        .iter()
        .filter_map(|(name, _)| shown.get(name).map(|value| (name.clone(), value.clone())))
        .collect();
    Ok(json!({"settings": listed, "calls": recorder.calls, "error": error}))
}

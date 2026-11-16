use std::collections::BTreeMap;

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

// The field names stay short because three routing_ keys trip clippy's shared-prefix lint
#[derive(Debug, Deserialize)]
struct Initial {
    #[serde(rename = "routing_strategy", default = "default_routing_strategy")]
    strategy: String,
    #[serde(rename = "routing_strategy_args", default)]
    args: Map<String, Value>,
    #[serde(rename = "routing_groups")]
    groups: Option<Value>,
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
    fn record(&mut self, step: &str, argument: Value) {
        self.calls.push(json!([step, argument]));
    }
}

impl RouterSettingsTarget for Recorder {
    fn set_optional_pre_call_checks(&mut self, checks: &Value) -> Step {
        self.record("set_optional_pre_call_checks", checks.clone());
        Ok(())
    }

    fn init_routing_strategy(&mut self, strategy: &Value, args: &Value) -> Step {
        let call = json!({"routing_strategy": strategy, "routing_strategy_args": args});
        self.record("routing_strategy_init", call);
        Ok(())
    }

    fn apply_updated_strategy_args(&mut self) -> Step {
        self.record("apply_updated_routing_strategy_args", Value::Null);
        Ok(())
    }

    fn apply_lar1(&mut self, args: Option<&Value>) -> Step {
        self.record("apply_lar1", args.cloned().unwrap_or(Value::Null));
        Ok(())
    }

    // The Python recorder validates the input as a list or None, which is where a bad value fails
    fn init_routing_groups(&mut self, input: Option<&Value>) -> Step {
        if input.is_some_and(|groups| !groups.is_array()) {
            return Err(RouterSettingsError::Target {
                name: "init_routing_groups".to_owned(),
                message: "routing_groups must be a list or null".to_owned(),
            });
        }
        self.record("init_routing_groups", input.cloned().unwrap_or(Value::Null));
        Ok(())
    }
}

// The values a Router built with no arguments stores for these names
fn seeded_settings(initial: &Initial) -> RouterRuntimeSettings {
    let mut values = BTreeMap::new();
    values.insert("routing_strategy".to_owned(), json!(initial.strategy));
    values.insert("routing_strategy_args".to_owned(), json!(initial.args));
    values.insert("allowed_fails".to_owned(), json!(3_i64));
    values.insert("cooldown_time".to_owned(), json!(5_i64));
    values.insert("num_retries".to_owned(), json!(2_i64));
    values.insert("retry_after".to_owned(), json!(0_i64));
    values.insert("retry_policy".to_owned(), Value::Null);
    values.insert("model_group_retry_policy".to_owned(), Value::Null);
    RouterRuntimeSettings {
        values,
        routing_groups_input: initial.groups.clone(),
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

// Every failure of update_settings belongs in the answer, so this step itself cannot fail
pub(super) fn run(input: &SettingsInput) -> Value {
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
    let listed: BTreeMap<String, Value> = input
        .kwargs
        .iter()
        .filter_map(|(name, _)| shown.get(name).map(|value| (name.clone(), value.clone())))
        .collect();
    json!({"settings": listed, "calls": recorder.calls, "error": error})
}

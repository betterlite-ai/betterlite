use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_bedrock::invoke_schema::{
    InvokeSchemaCapabilities, apply_structured_output, ensure_tool_names,
    normalize_schema_custom_types, normalize_tool_custom_field, normalize_tool_schema_types,
    strip_eager_input_streaming, strip_unsupported_config_keys,
};
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{Event, Level, Subscriber, subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use tokio::runtime::{Builder, Runtime};

mod tests;

const EFFORT_FLAGS: [&str; 7] = [
    "supports_output_config",
    "supports_low_reasoning_effort",
    "supports_minimal_reasoning_effort",
    "supports_medium_reasoning_effort",
    "supports_high_reasoning_effort",
    "supports_xhigh_reasoning_effort",
    "supports_max_reasoning_effort",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    model_cost: BTreeMap<String, Object>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Step {
    ApplyStructuredOutput { model: String, body: Object },
    StripUnsupportedConfigKeys { model: String, body: Object },
    NormalizeCustomField { body: Object },
    ToolsWithoutEagerStreaming { body: Object },
    NormalizeSchema { schema: Json },
    NormalizeToolSchemas { body: Object },
    EnsureToolNames { body: Object },
}

// The case fixture answers the two capability questions the production trait asks.
struct ModelCost<'fixture> {
    entries: &'fixture BTreeMap<String, Object>,
}

impl ModelCost<'_> {
    fn flag(&self, model: &str, key: &str) -> bool {
        self.entries
            .get(model)
            .is_some_and(|entry| matches!(entry.get(key), Some(Json::Bool(true))))
    }
}

impl InvokeSchemaCapabilities for ModelCost<'_> {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.flag(request.model, key))
    }

    fn supports_effort_param(&mut self, model: &str) -> impl Future<Output = bool> + Send {
        ready(EFFORT_FLAGS.iter().any(|flag| self.flag(model, flag)))
    }
}

#[derive(Clone, Default)]
struct Warnings {
    events: Arc<AtomicUsize>,
}

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        self.events.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut catalog = ModelCost {
            entries: &input.model_cost,
        };
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let subscriber_guard = subscriber::set_default(subscriber);
        let mut answers = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            answers.push(answer(step, &mut catalog, &runtime, &warnings));
        }
        drop(subscriber_guard);
        Ok(Value::Array(answers))
    }
}

fn answer(
    step: &Step,
    catalog: &mut ModelCost<'_>,
    runtime: &Runtime,
    warnings: &Warnings,
) -> Value {
    match step {
        Step::ApplyStructuredOutput { model, body } => {
            warned_answer(body, warnings, |mutable_body| {
                runtime.block_on(apply_structured_output(model, mutable_body, catalog));
            })
        }
        Step::StripUnsupportedConfigKeys { model, body } => {
            warned_answer(body, warnings, |mutable_body| {
                runtime.block_on(strip_unsupported_config_keys(model, mutable_body, catalog));
            })
        }
        Step::NormalizeCustomField { body } => object_answer(body, normalize_tool_custom_field),
        Step::NormalizeToolSchemas { body } => object_answer(body, normalize_tool_schema_types),
        Step::EnsureToolNames { body } => object_answer(body, ensure_tool_names),
        Step::ToolsWithoutEagerStreaming { body } => {
            json!({"tools": strip_eager_input_streaming(body)})
        }
        Step::NormalizeSchema { schema } => {
            let mut owned_schema = schema.clone();
            normalize_schema_custom_types(&mut owned_schema);
            json!({"schema": owned_schema})
        }
    }
}

fn object_answer(body: &Object, run: fn(&mut Object)) -> Value {
    let mut owned_body = body.clone();
    run(&mut owned_body);
    json!({"body": owned_body})
}

fn warned_answer<Run>(body: &Object, warnings: &Warnings, run: Run) -> Value
where
    Run: FnOnce(&mut Object),
{
    let mut owned_body = body.clone();
    let before = warnings.events.load(Ordering::Relaxed);
    run(&mut owned_body);
    let warned = warnings.events.load(Ordering::Relaxed) > before;
    json!({"body": owned_body, "warned": warned})
}

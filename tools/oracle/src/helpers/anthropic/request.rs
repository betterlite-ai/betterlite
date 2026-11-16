use betterlite_anthropic::request::{BillingHeader, PreparedRequest};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use betterlite_oracle_helper_api::error::OrHelperError as _;

mod fixtures;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use fixtures::warnings::Warnings;
use fixtures::{FixtureInput, Records};
use tokio::runtime::Builder;
use tracing::subscriber;

// LiteLLM stores the sanitized-to-original tool name map here and only when a name was rewritten
const REVERSE_MAP_KEY: &str = "_anthropic_tool_name_map";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: FixtureInput,
    model: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    optional_params: IndexMap<String, Json>,
    #[serde(default)]
    litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    headers: IndexMap<String, String>,
    #[serde(default)]
    strips_billing_metadata: BillingHeader,
    #[serde(default)]
    modify_params: bool,
    #[serde(default)]
    global_drop_params: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let records = Records::load(&input.fixture).or_config_error()?;
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        subscriber::with_default(subscriber, || {
            runtime.block_on(answer(input, &records, &warnings))
        })
    }
}

async fn answer(input: &Input, records: &Records, warnings: &Warnings) -> HelperResult {
    let prepared = match records.prepare(input).await {
        Ok(prepared) => prepared,
        Err(error) => return Ok(json!({"raised": true, "status": error.status_code()})),
    };
    let reverse = reverse_map(input, &prepared);
    Ok(json!({
        "body": prepared.body,
        "headers": prepared.headers.values,
        "reverse": reverse,
        "warning": warnings.first(),
    }))
}

// A rewrite replaces the stored map; otherwise the caller's own map stays untouched, as in LiteLLM
fn reverse_map(input: &Input, prepared: &PreparedRequest) -> Value {
    if prepared.tool_name_reverse.is_empty() {
        return input
            .litellm_params
            .get(REVERSE_MAP_KEY)
            .map_or(Value::Null, |value| json!(value));
    }
    json!(prepared.tool_name_reverse)
}

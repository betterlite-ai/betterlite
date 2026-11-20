use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::guardrails::anthropic_translation::structured_write_back::{
    fold_leading_systems, hoisted_system_message, system_row_to_anthropic,
    write_back_structured_messages,
};
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Runtime};

use super::requests::Scenario;

fn current_runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_config_error()
}

pub(super) fn hoisted_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let runtime = current_runtime()?;
    let read = hoisted_system_message(&scenario.data);
    let message = runtime.block_on(read).or_value_error()?;
    Ok(json!({"message": message.unwrap_or(Value::Null)}))
}

pub(super) fn system_conversion_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let message = Value::Object(scenario.message.clone());
    let row = system_row_to_anthropic(&message);
    Ok(json!({"row": row.map_or(Value::Null, Value::Object)}))
}

pub(super) fn fold_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let mut data = scenario.data.clone();
    fold_leading_systems(&mut data, &scenario.leading, scenario.include_existing);
    Ok(json!({"data": data}))
}

pub(super) fn structured_answer(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let runtime = current_runtime()?;
    let mut data = scenario.data.clone();
    runtime.block_on(write_structured(&mut data, scenario))?;
    Ok(json!({"data": data}))
}

async fn write_structured(
    data: &mut Map<String, Value>,
    scenario: &Scenario,
) -> Result<(), Box<HelperError>> {
    let hoisted = if scenario.compute_hoisted {
        hoisted_system_message(data).await.or_value_error()?
    } else {
        None
    };
    write_back_structured_messages(
        data,
        &scenario.messages,
        hoisted.as_ref(),
        scenario.preserve_system_messages,
    )
    .await
    .or_value_error()
}

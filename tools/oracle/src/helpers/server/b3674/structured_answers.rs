use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::guardrails::anthropic_translation::structured_write_back::{
    defer_systems_inside_tool_exchanges, fold_leading_systems_into_top_level,
    hoisted_top_level_system_message, openai_system_message_to_anthropic,
    write_back_structured_messages,
};
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

use super::requests::{
    DeferScenario, FoldScenario, HoistedScenario, StructuredScenario, SystemConversionScenario,
};

pub(super) fn hoisted_answer(scenario: &HoistedScenario) -> Result<Value, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    let message = runtime
        .block_on(hoisted_top_level_system_message(&scenario.data))
        .or_value_error()?;
    Ok(json!({"message": message.unwrap_or(Value::Null)}))
}

pub(super) fn system_conversion_answer(
    scenario: &SystemConversionScenario,
) -> Result<Value, Box<HelperError>> {
    let row = openai_system_message_to_anthropic(&scenario.message);
    Ok(json!({"row": row.map_or(Value::Null, Value::Object)}))
}

pub(super) fn fold_answer(scenario: &FoldScenario) -> Result<Value, Box<HelperError>> {
    let mut data = scenario.data.clone();
    fold_leading_systems_into_top_level(&mut data, &scenario.leading, scenario.include_existing);
    Ok(json!({"data": data}))
}

pub(super) fn defer_answer(scenario: &DeferScenario) -> Result<Value, Box<HelperError>> {
    Ok(json!({"messages": defer_systems_inside_tool_exchanges(&scenario.messages)}))
}

pub(super) fn structured_answer(scenario: &StructuredScenario) -> Result<Value, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    let mut data = scenario.data.clone();
    runtime.block_on(write_structured(&mut data, scenario))?;
    Ok(json!({"data": data}))
}

async fn write_structured(
    data: &mut Map<String, Value>,
    scenario: &StructuredScenario,
) -> Result<(), Box<HelperError>> {
    let hoisted = if scenario.compute_hoisted {
        hoisted_top_level_system_message(data)
            .await
            .or_value_error()?
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

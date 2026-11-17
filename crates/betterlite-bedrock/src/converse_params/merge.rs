use crate::converse_params::wire::carries_value;
use crate::model_routes::{ParamMap, ParamValue};

const AUTO: &str = "auto";
const DISABLE_PARALLEL_FIELD: &str = "disable_parallel_tool_use";
const TOOL_CHOICE_FIELD: &str = "tool_choice";
const TYPE_FIELD: &str = "type";

// The internal block LiteLLM stores for parallel_tool_calls before the request fields are assembled.
#[must_use]
pub fn parallel_tool_use_config(value: &ParamValue) -> ParamMap {
    let mut tool_choice = ParamMap::new();
    tool_choice.insert(TYPE_FIELD.to_owned(), ParamValue::Text(AUTO.to_owned()));
    tool_choice.insert(
        DISABLE_PARALLEL_FIELD.to_owned(),
        ParamValue::Bool(!carries_value(value)),
    );
    ParamMap::from([(
        TOOL_CHOICE_FIELD.to_owned(),
        ParamValue::Mapping(tool_choice),
    )])
}

// Generated fields land first, the caller's fields overlay them, then the generated non-type fields win.
#[must_use]
pub fn merge_parallel_tool_use_config(
    additional_request_params: &ParamMap,
    parallel_tool_use_config: &ParamMap,
) -> ParamMap {
    let mut merged = additional_request_params.clone();
    for (key, generated) in parallel_tool_use_config {
        let entry = merge_entry(generated, additional_request_params.get(key));
        merged.insert(key.clone(), entry);
    }
    merged
}

fn merge_entry(generated: &ParamValue, existing: Option<&ParamValue>) -> ParamValue {
    let (ParamValue::Mapping(generated_fields), Some(ParamValue::Mapping(existing_fields))) =
        (generated, existing)
    else {
        return generated.clone();
    };
    let mut combined = generated_fields.clone();
    combined.extend(existing_fields.clone());
    for (name, value) in generated_fields {
        if name != TYPE_FIELD {
            combined.insert(name.clone(), value.clone());
        }
    }
    ParamValue::Mapping(combined)
}

// Converse rejects both toolConfig.toolChoice and additionalModelRequestFields.tool_choice.type.
pub fn drop_tool_choice_type(additional_request_params: &mut ParamMap) {
    if let Some(ParamValue::Mapping(tool_choice)) =
        additional_request_params.get_mut(TOOL_CHOICE_FIELD)
    {
        tool_choice.shift_remove(TYPE_FIELD);
    }
}

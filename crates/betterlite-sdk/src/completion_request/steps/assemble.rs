use super::convert::{builtin_messages, convert, messages_from_builtin, typed_from};
use super::messages::optional;
use super::params::{drop_params_flag, provider_name};
use crate::completion_request::Preparation;
use crate::completion_request::args::JsonObject;
use crate::completion_request::error::{CompletionRequestError, RequestResult};
use crate::completion_request::ports::ParameterMappingRequest;
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::function_fallback::pre_process_optional_params;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_context::non_default_params::pre_process::{
    PreProcessOptions, PreProcessRequest, pre_process_non_default_params,
};
use betterlite_models::decisions::Json;
use betterlite_models::params::{ChatParameters, ParameterPolicy};
use betterlite_prompts::builtin::Field;
use betterlite_prompts::factory::custom::function_call_prompt;
use serde::Serialize;
fn override_field<T>(params: &mut JsonObject, name: &str, value: Option<&T>) -> RequestResult<()>
where
    T: Serialize + ?Sized,
{
    params.shift_remove(name);
    if let Some(given) = value {
        params.insert(
            name.to_owned(),
            serde_json::from_value(serde_json::to_value(given)?)?,
        );
    }
    Ok(())
}
fn named_arguments(state: &Preparation<'_>) -> RequestResult<JsonObject> {
    let encoded = serde_json::to_value(state.args)?;
    let mut params: JsonObject = serde_json::from_value(encoded)?;
    params.shift_remove("model_list");
    params.shift_remove("kwargs");
    params.retain(|_name, value| !matches!(value, Json::Null));
    params.insert(String::from("model"), Json::Text(state.call.model.clone()));
    override_field(
        &mut params,
        "custom_llm_provider",
        state.call.custom_llm_provider.as_deref(),
    )?;
    override_field(&mut params, "tools", state.call.tools.as_ref())?;
    override_field(&mut params, "stop", state.stop.as_ref())?;
    override_field(&mut params, "tool_choice", state.tool_choice.as_ref())?;
    override_field(&mut params, "thinking", state.thinking.as_ref())?;
    override_field(&mut params, "max_retries", state.call.max_retries.as_ref())?;
    override_field(&mut params, "base_model", state.call.base_model.as_ref())?;
    Ok(params)
}
struct MappingInputs {
    named: Params,
    extra: Params,
    additional_drop: Vec<String>,
    allowed: Vec<String>,
    provider: String,
}
fn list_inputs(kwargs: &JsonObject) -> RequestResult<(Vec<String>, Vec<String>)> {
    let drop = optional::<Vec<String>>(kwargs, "additional_drop_params")?.unwrap_or_default();
    let allowed = optional::<Vec<String>>(kwargs, "allowed_openai_params")?.unwrap_or_default();
    Ok((drop, allowed))
}
fn parameter_policy(state: &Preparation<'_>, inputs: &MappingInputs) -> ParameterPolicy {
    let dropped = state.settings.drop_params == Some(true)
        || drop_params_flag(&state.kwargs).unwrap_or(false);
    ParameterPolicy {
        drop_params: dropped,
        additional_drop_params: inputs.additional_drop.clone(),
        allowed_openai_params: inputs.allowed.clone(),
    }
}
fn chat_parameters(params: &Params) -> RequestResult<ChatParameters> {
    let object = convert::<JsonObject, _>(params)?;
    let encoded = serde_json::to_value(&object)?;
    serde_json::from_value(encoded).map_err(|source| CompletionRequestError::Field {
        field: "parameters",
        source,
    })
}
fn apply_function_fallback(
    state: &mut Preparation<'_>,
    params: &mut Params,
    pre_adds: bool,
) -> RequestResult<()> {
    if !(state.settings.flags.add_function_to_prompt || pre_adds) {
        return Ok(());
    }
    let Some(given) = params.remove("functions_unsupported_model") else {
        return Ok(());
    };
    let functions: Vec<Field> =
        typed_from(&convert::<Json, _>(&given)?, "functions_unsupported_model")?;
    let messages = builtin_messages(&state.call.messages)?;
    state.call.messages = messages_from_builtin(&function_call_prompt(&messages, &functions)?)?;
    Ok(())
}
fn map_options(
    state: &mut Preparation<'_>,
    inputs: &MappingInputs,
    mapper: Option<&dyn OptionalParamsMapper>,
) -> RequestResult<()> {
    let policy = parameter_policy(state, inputs);
    let mut special = pre_process_non_default_params(
        &PreProcessRequest {
            passed_params: &inputs.named,
            special_params: &inputs.extra,
            custom_llm_provider: &inputs.provider,
            additional_drop_params: &inputs.additional_drop,
            provider_config: mapper,
        },
        PreProcessOptions {
            remove_sensitive_keys: false,
            add_provider_specific_params: false,
        },
    )?;
    let optional = pre_process_optional_params(&inputs.named, &mut special, &inputs.provider);
    let mapping = state
        .ports
        .model_metadata
        .parameter_mapping(&ParameterMappingRequest {
            model: &state.call.model,
            custom_llm_provider: &inputs.provider,
            base_model: state.call.base_model.as_deref(),
            parameters: &chat_parameters(&special)?,
            policy: &policy,
        })?;
    let standard = serde_json::to_value(&mapping.standard)?;
    let mut params: Params = serde_json::from_value(standard)?;
    let fields = serde_json::to_value(&mapping.provider_fields)?;
    params.extend(serde_json::from_value::<Params>(fields)?);
    state.call.options.dropped_fields = mapping.dropped_fields;
    let mut optional_params = optional.optional_params;
    apply_function_fallback(state, &mut optional_params, optional.add_function_to_prompt)?;
    params.extend(optional_params);
    state.call.options.params = params;
    Ok(())
}
fn processed_non_default(
    inputs: &MappingInputs,
    mapper: Option<&dyn OptionalParamsMapper>,
) -> RequestResult<JsonObject> {
    let processed = pre_process_non_default_params(
        &PreProcessRequest {
            passed_params: &inputs.named,
            special_params: &inputs.extra,
            custom_llm_provider: &inputs.provider,
            additional_drop_params: &inputs.additional_drop,
            provider_config: mapper,
        },
        PreProcessOptions {
            remove_sensitive_keys: true,
            add_provider_specific_params: true,
        },
    )?;
    convert::<JsonObject, _>(&processed)
}
pub(crate) fn assemble(state: &mut Preparation<'_>) -> RequestResult<()> {
    let (additional_drop, allowed) = list_inputs(&state.kwargs)?;
    let inputs = MappingInputs {
        named: convert::<Params, _>(&named_arguments(state)?)?,
        extra: convert::<Params, _>(&state.non_default)?,
        additional_drop,
        allowed,
        provider: provider_name(state),
    };
    let chat_policy = state.ports.model_metadata.chat_policy(
        &state.call.model,
        &inputs.provider,
        state.call.base_model.as_deref(),
    );
    map_options(state, &inputs, chat_policy.mapper)?;
    state.call.non_default_params = processed_non_default(&inputs, chat_policy.mapper)?;
    Ok(())
}

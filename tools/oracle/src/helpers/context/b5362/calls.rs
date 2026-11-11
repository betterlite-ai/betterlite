use std::convert::Infallible;

use betterlite_context::media_optional_params::image_generation::{
    ImageGenerationRequest, get_optional_params_image_gen,
};
use betterlite_context::media_optional_params::transcription::{
    TranscriptionRequest, transcription_optional_params,
};
use betterlite_context::non_default_params::{
    extra_body, filters, function_fallback, mapper, pre_process, reasoning_aliases,
};
use serde::Serialize;
use serde_json::{Value, json, to_value};

use super::{Call, Failure, Function, required};

fn answer<T: Serialize>(result: Result<T, u16>) -> Result<Value, Failure> {
    match result {
        Ok(value) => Ok(to_value(value)?),
        Err(status) => Err(Failure::Status(status)),
    }
}

fn drop_policy(call: &Call) -> mapper::DropPolicy {
    mapper::DropPolicy {
        drop_params: call.drop_params,
        litellm_drop_params: call.litellm_drop_params.unwrap_or(false),
    }
}

fn fixture_mapper(spec: Option<&super::MapperSpec>) -> Option<&dyn mapper::OptionalParamsMapper> {
    spec.map(|fixture| -> &dyn mapper::OptionalParamsMapper { fixture })
}

fn named_params(call: &Call) -> Result<Value, Failure> {
    if matches!(call.function, Function::SelectNonDefault) {
        let names: Vec<&str> = call.default_names.iter().map(String::as_str).collect();
        let drops = call.additional_drop_params.as_deref().unwrap_or_default();
        return Ok(to_value(filters::select_non_default_params(
            &call.passed_params,
            &names,
            drops,
        ))?);
    }
    let excluding: Vec<&str> = call.excluding.iter().map(String::as_str).collect();
    Ok(to_value(filters::filter_out_litellm_params(
        &call.kwargs,
        &excluding,
    ))?)
}

fn route_for_request(call: &Call) -> Result<Value, Failure> {
    let model = required(call.model.as_deref(), "model")?;
    let (routed, filtered) = filters::bedrock_route_for_request(
        model,
        &call.passed_params,
        call.additional_drop_params.as_deref().unwrap_or_default(),
        |name, kept| Ok::<_, Infallible>((name.to_owned(), kept.clone())),
    )
    .unwrap_or_else(|never| match never {});
    Ok(json!({ "model": routed, "params": filtered }))
}

fn pre_process_call(call: &Call) -> Result<Value, Failure> {
    if matches!(call.function, Function::RemoveSensitiveKeys) {
        let mut params = call.params.clone();
        pre_process::remove_sensitive_keys(&mut params);
        return Ok(to_value(params)?);
    }
    let fields = required(call.request.as_ref(), "request")?;
    let request = pre_process::PreProcessRequest {
        passed_params: &fields.passed_params,
        special_params: &fields.special_params,
        custom_llm_provider: &fields.custom_llm_provider,
        additional_drop_params: fields.additional_drop_params.as_deref().unwrap_or_default(),
    };
    let options = pre_process::PreProcessOptions {
        remove_sensitive_keys: call.remove_sensitive_keys.unwrap_or(false),
        add_provider_specific_params: call.add_provider_specific_params.unwrap_or(false),
    };
    answer(pre_process::pre_process_non_default_params(&request, options).map_err(|error| {
        error.status_code()
    }))
}

fn fallback_params(call: &Call) -> Result<Value, Failure> {
    let provider = required(call.custom_llm_provider.as_deref(), "custom_llm_provider")?;
    if matches!(call.function, Function::FunctionFallback) {
        let mut remaining = call.non_default_params.clone();
        let processed = function_fallback::pre_process_optional_params(
            &call.passed_params,
            &mut remaining,
            provider,
        );
        return Ok(json!({
            "optional_params": processed.optional_params,
            "non_default_params": remaining,
            "add_function_to_prompt": processed.add_function_to_prompt,
        }));
    }
    let openai_params: Vec<&str> = call.openai_params.iter().map(String::as_str).collect();
    let request = extra_body::ProviderSpecificRequest {
        passed_params: &call.passed_params,
        custom_llm_provider: provider,
        openai_params: &openai_params,
        additional_drop_params: call.additional_drop_params.as_deref().unwrap_or_default(),
    };
    answer(
        extra_body::add_provider_specific_params(call.optional_params.clone(), &request)
            .map_err(|error| error.status_code()),
    )
}

fn alias_params(call: &Call) -> Result<Value, Failure> {
    if matches!(call.function, Function::PeekAliases) {
        return Ok(to_value(
            reasoning_aliases::peek_reasoning_summary_aliases(&call.optional_params),
        )?);
    }
    let (stripped, summary) = reasoning_aliases::strip_reasoning_aliases(&call.optional_params);
    Ok(json!({ "params": stripped, "summary": summary }))
}

fn media_params(call: &Call) -> Result<Value, Failure> {
    if matches!(call.function, Function::ImageGeneration) {
        let request = ImageGenerationRequest {
            model: call.model.as_deref(),
            custom_llm_provider: call.custom_llm_provider.as_deref(),
            params: &call.params,
            additional_drop_params: call.additional_drop_params.as_deref().unwrap_or_default(),
            drop_policy: drop_policy(call),
            provider_config: fixture_mapper(call.provider_config.as_ref()),
            bedrock_config: fixture_mapper(call.bedrock.as_ref()),
        };
        return answer(get_optional_params_image_gen(&request).map_err(|error| error.status_code()));
    }
    let request = TranscriptionRequest {
        model: required(call.model.as_deref(), "model")?,
        custom_llm_provider: required(call.custom_llm_provider.as_deref(), "custom_llm_provider")?,
        params: &call.params,
        drop_policy: drop_policy(call),
        mapper: fixture_mapper(call.mapper.as_ref()),
    };
    answer(transcription_optional_params(&request).map_err(|error| error.status_code()))
}

pub(super) fn run(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::SelectNonDefault | Function::FilterOutLitellm => named_params(call),
        Function::BedrockRoute => route_for_request(call),
        Function::PreProcess | Function::RemoveSensitiveKeys => pre_process_call(call),
        Function::FunctionFallback | Function::AddProviderSpecific => fallback_params(call),
        Function::PeekAliases | Function::StripAliases => alias_params(call),
        Function::Transcription | Function::ImageGeneration => media_params(call),
    }
}

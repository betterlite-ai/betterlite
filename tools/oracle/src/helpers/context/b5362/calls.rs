use std::convert::Infallible;

use betterlite_context::media_optional_params::image_generation::{
    ImageGenerationRequest, get_optional_params_image_gen,
};
use betterlite_context::media_optional_params::transcription::{
    TranscriptionRequest, transcription_optional_params,
};
use betterlite_context::non_default_params::function_fallback::pre_process_optional_params;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper as Cfg;
use betterlite_context::non_default_params::reasoning_aliases::{
    peek_reasoning_summary_aliases, strip_reasoning_aliases,
};
use betterlite_context::non_default_params::{extra_body, filters, mapper, pre_process};
use serde::Serialize;
use serde_json::{Value, json, to_value};

use super::{Call, Failure, Function, required};

fn answer<T>(result: Result<T, u16>) -> Result<Value, Failure>
where
    T: Serialize,
{
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
    let fields = required(call.request.as_ref(), "request")?;
    let request = pre_process::PreProcessRequest {
        passed_params: &fields.passed_params,
        special_params: &fields.special_params,
        custom_llm_provider: &fields.custom_llm_provider,
        additional_drop_params: fields.additional_drop_params.as_deref().unwrap_or_default(),
        provider_config: call.provider_config.as_ref().map(|fx| -> &dyn Cfg { fx }),
    };
    let options = pre_process::PreProcessOptions {
        remove_sensitive_keys: call.remove_sensitive_keys.unwrap_or(false),
        add_provider_specific_params: call.add_provider_specific_params.unwrap_or(false),
    };
    let processed = pre_process::pre_process_non_default_params(&request, options);
    answer(processed.map_err(|error| error.status_code()))
}

fn add_provider_specific(call: &Call) -> Result<Value, Failure> {
    let openai_params: Vec<&str> = call.openai_params.iter().map(String::as_str).collect();
    let request = extra_body::ProviderSpecificRequest {
        passed_params: &call.passed_params,
        custom_llm_provider: required(call.custom_llm_provider.as_deref(), "custom_llm_provider")?,
        openai_params: &openai_params,
        additional_drop_params: call.additional_drop_params.as_deref().unwrap_or_default(),
    };
    let added = extra_body::add_provider_specific_params(call.optional_params.clone(), &request);
    answer(added.map_err(|error| error.status_code()))
}

fn media_params(call: &Call) -> Result<Value, Failure> {
    if matches!(call.function, Function::ImageGeneration) {
        let request = ImageGenerationRequest {
            model: call.model.as_deref(),
            custom_llm_provider: call.custom_llm_provider.as_deref(),
            params: &call.params,
            additional_drop_params: call.additional_drop_params.as_deref().unwrap_or_default(),
            drop_policy: drop_policy(call),
            provider_config: call.provider_config.as_ref().map(|fx| -> &dyn Cfg { fx }),
            bedrock_config: call.bedrock.as_ref().map(|fx| -> &dyn Cfg { fx }),
        };
        return answer(
            get_optional_params_image_gen(&request).map_err(|error| error.status_code()),
        );
    }
    let request = TranscriptionRequest {
        model: required(call.model.as_deref(), "model")?,
        custom_llm_provider: required(call.custom_llm_provider.as_deref(), "custom_llm_provider")?,
        params: &call.params,
        drop_policy: drop_policy(call),
        mapper: call.mapper.as_ref().map(|fx| -> &dyn Cfg { fx }),
    };
    answer(transcription_optional_params(&request).map_err(|error| error.status_code()))
}

fn ollama_fallback(call: &Call) -> Result<Value, Failure> {
    let mut remaining = call.non_default_params.clone();
    let provider = required(call.custom_llm_provider.as_deref(), "custom_llm_provider")?;
    let done = pre_process_optional_params(&call.passed_params, &mut remaining, provider);
    let (optional_params, add_function_to_prompt) =
        (done.optional_params, done.add_function_to_prompt);
    Ok(json!({
        "optional_params": optional_params,
        "non_default_params": remaining,
        "add_function_to_prompt": add_function_to_prompt,
    }))
}

fn select_non_default(call: &Call) -> Result<Value, Failure> {
    let names: Vec<&str> = call.default_names.iter().map(String::as_str).collect();
    let drops = call.additional_drop_params.as_deref().unwrap_or_default();
    let selected = filters::select_non_default_params(&call.passed_params, &names, drops);
    Ok(to_value(selected)?)
}

pub(super) fn run(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::SelectNonDefault => select_non_default(call),
        Function::FilterOutLitellm => {
            let excluding: Vec<&str> = call.excluding.iter().map(String::as_str).collect();
            let kept = call.endpoint.map_or_else(
                || filters::filter_out_litellm_params(&call.kwargs, &excluding),
                |endpoint| filters::non_default_kwargs(&call.kwargs, endpoint),
            );
            Ok(to_value(kept)?)
        }
        Function::RemoveSensitiveKeys => {
            let mut params = call.params.clone();
            pre_process::remove_sensitive_keys(&mut params);
            Ok(to_value(params)?)
        }
        Function::PeekAliases => Ok(to_value(peek_reasoning_summary_aliases(
            &call.optional_params,
        ))?),
        Function::StripAliases => {
            let (stripped, summary) = strip_reasoning_aliases(&call.optional_params);
            Ok(json!({ "params": stripped, "summary": summary }))
        }
        Function::BedrockRoute => route_for_request(call),
        Function::PreProcess => pre_process_call(call),
        Function::OllamaFallback => ollama_fallback(call),
        Function::AddProviderSpecific => add_provider_specific(call),
        Function::Transcription | Function::ImageGeneration => media_params(call),
    }
}

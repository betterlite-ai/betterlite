use std::collections::BTreeMap;

use betterlite_openai::chat_content::{Mode, RefuseDownloads};
use betterlite_openai::openai_base_config::{CatalogCapabilities, Headers, Object as WireObject};
use betterlite_openai::small_chat_hosts::CompletionTokenRename::Keep;
use betterlite_openai::small_chat_hosts::meta_llama::LlamaAPIConfig;
use betterlite_openai::small_chat_hosts::nebius::NebiusConfig;
use betterlite_openai::small_chat_hosts::wandb::WandbConfig;
use betterlite_openai::small_chat_hosts::zai::ZaiChatConfig;
use betterlite_openai::small_chat_hosts::{
    map_supported, maritalk, moonshot, morph, nadir, novita, nscale, openai_parent, v0,
};
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use calls::{
    CacheCall, CostCall, ErrorCall, Input, Lookup, NovitaCall, ParamsCall, Provider, RequestCall,
    SupportedCall, UrlCall, capabilities,
};

mod calls;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!({
            "supported": input.supported.iter().map(supported_record).collect::<Vec<_>>(),
            "params": input.params.iter().map(params_record).collect::<Vec<_>>(),
            "moonshot_urls": input.moonshot_urls.iter().map(url_record).collect::<Vec<_>>(),
            "moonshot_requests": input.moonshot_requests.iter().map(request_record).collect::<Vec<_>>(),
            "nadir_cost": input.nadir_cost.iter().map(cost_record).collect::<Vec<_>>(),
            "novita_headers": input.novita_headers.iter().map(novita_record).collect::<Vec<_>>(),
            "zai_cache": input.zai_cache.iter().map(cache_record).collect::<Vec<_>>(),
            "maritalk_errors": input.maritalk_errors.iter().map(error_record).collect::<Vec<_>>(),
        }))
    }
}

fn record(status: Result<Value, Option<u16>>) -> Value {
    match status {
        Ok(value) => json!({"ok": true, "value": value, "status": null}),
        Err(code) => json!({"ok": false, "value": null, "status": code}),
    }
}

fn supported_list(
    provider: Provider,
    model: &str,
    reasoning: bool,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str> {
    let lookup = Lookup(reasoning);
    match provider {
        Provider::Maritalk => maritalk::MaritalkConfig::supported_params(),
        Provider::Llama => LlamaAPIConfig::supported_params(model, catalog),
        Provider::Moonshot => moonshot::supported_params(&lookup, model, catalog),
        Provider::Morph => morph::MorphChatConfig::supported_params(model),
        Provider::Nadir => nadir::NadirConfig::supported_params(),
        Provider::Nebius => NebiusConfig::supported_params(model, catalog),
        Provider::Novita => openai_parent().supported_params(model, catalog),
        Provider::Nscale => nscale::NscaleConfig::supported_params(model),
        Provider::V0 => v0::V0ChatConfig::supported_params(model),
        Provider::Wandb => WandbConfig::supported_params(&lookup, model, catalog),
        Provider::Zai => ZaiChatConfig::supported_params(&lookup, model),
    }
}

fn supported_record(call: &SupportedCall) -> Value {
    let catalog = capabilities(&call.chat_models, &call.text_models);
    let params = supported_list(call.provider, &call.model, call.reasoning, &catalog);
    record(Ok(Value::Array(
        params.into_iter().map(Value::from).collect(),
    )))
}

fn wire_object(object: &Object) -> WireObject {
    object
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                serde_json::to_value(value).unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn params_record(call: &ParamsCall) -> Value {
    let catalog = capabilities(&call.chat_models, &call.text_models);
    let supported = supported_list(call.provider, &call.model, call.reasoning, &catalog);
    record(Ok(mapped_params(call, &catalog, &supported)))
}

fn mapped_params(call: &ParamsCall, catalog: &CatalogCapabilities, supported: &[&str]) -> Value {
    match call.provider {
        Provider::Llama => llama_mapped(call, catalog),
        Provider::Moonshot => moonshot_mapped(call, catalog),
        Provider::Nebius => nebius_mapped(call, catalog),
        Provider::Wandb => wandb_mapped(call, catalog),
        Provider::Maritalk
        | Provider::Morph
        | Provider::Nadir
        | Provider::Novita
        | Provider::Nscale
        | Provider::V0
        | Provider::Zai => inherited_mapped(call, supported),
    }
}

fn llama_mapped(call: &ParamsCall, catalog: &CatalogCapabilities) -> Value {
    let mut wire = wire_object(&call.existing);
    let requested = wire_object(&call.requested);
    LlamaAPIConfig::map_openai_params(&requested, &mut wire, &call.model, catalog);
    serde_json::to_value(wire).unwrap_or(Value::Null)
}

fn moonshot_mapped(call: &ParamsCall, catalog: &CatalogCapabilities) -> Value {
    let lookup = Lookup(call.reasoning);
    let mut optional = call.existing.clone();
    let model = call.model.as_str();
    moonshot::map_openai_params(&lookup, model, &call.requested, &mut optional, catalog);
    serde_json::to_value(optional).unwrap_or(Value::Null)
}

fn nebius_mapped(call: &ParamsCall, catalog: &CatalogCapabilities) -> Value {
    let mut optional = call.existing.clone();
    NebiusConfig::map_openai_params(&call.model, &call.requested, &mut optional, catalog);
    serde_json::to_value(optional).unwrap_or(Value::Null)
}

fn wandb_mapped(call: &ParamsCall, catalog: &CatalogCapabilities) -> Value {
    let lookup = Lookup(call.reasoning);
    let mut optional = call.existing.clone();
    let model = call.model.as_str();
    WandbConfig::map_openai_params(&lookup, model, &call.requested, &mut optional, catalog);
    serde_json::to_value(optional).unwrap_or(Value::Null)
}

fn inherited_mapped(call: &ParamsCall, supported: &[&str]) -> Value {
    let mut optional = call.existing.clone();
    map_supported(&call.requested, &mut optional, supported, Keep);
    serde_json::to_value(optional).unwrap_or(Value::Null)
}

fn url_record(call: &UrlCall) -> Value {
    record(Ok(Value::from(moonshot::complete_url(
        call.api_base.as_deref(),
    ))))
}

fn request_record(call: &RequestCall) -> Value {
    let mut optional = call.optional_params.clone();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let downloads = RefuseDownloads;
    let input = moonshot::TransformInput {
        model: &call.model,
        messages: &call.messages,
        optional_params: &mut optional,
        environment: &environment,
        downloads: &downloads,
    };
    match moonshot::transform_request(&Lookup(call.reasoning), input, Mode::Sync) {
        Ok(body) => record(Ok(serde_json::to_value(body).unwrap_or(Value::Null))),
        Err(error) => record(Err(Some(error.status_code()))),
    }
}

fn cost_record(call: &CostCall) -> Value {
    record(Ok(
        nadir::nadir_reported_cost(call.body.as_bytes()).map_or(Value::Null, |value| json!(value))
    ))
}

fn novita_record(call: &NovitaCall) -> Value {
    let key = call
        .api_key
        .as_deref()
        .map(|text| SecretString::from(text.to_owned()));
    let mut headers = Headers::new();
    for (name, value) in &call.headers {
        headers.insert(name.clone(), value.clone());
    }
    match novita::NovitaConfig::validate_environment(key.as_ref(), &headers) {
        Ok(validated) => record(Ok(serde_json::to_value(validated).unwrap_or(Value::Null))),
        Err(_error) => record(Err(None)),
    }
}

fn cache_record(call: &CacheCall) -> Value {
    // ast-grep-ignore: no-long-names - LiteLLM's method name
    let (messages, tools) = ZaiChatConfig::remove_cache_control_flag_from_messages_and_tools(
        &call.model,
        &call.messages,
        call.tools.as_deref(),
    );
    record(Ok(json!({
        "messages": serde_json::to_value(messages).unwrap_or(Value::Null),
        "tools": serde_json::to_value(tools).unwrap_or(Value::Null),
    })))
}

fn error_record(call: &ErrorCall) -> Value {
    let error = maritalk::MaritalkConfig::get_error_class(
        &call.message,
        call.status,
        call.headers.as_ref(),
    );
    let headers: Option<BTreeMap<String, String>> =
        error.error.headers.map(|pairs| pairs.into_iter().collect());
    record(Ok(json!({
        "status_code": error.error.status_code,
        "message": error.error.message,
        "headers": headers,
    })))
}

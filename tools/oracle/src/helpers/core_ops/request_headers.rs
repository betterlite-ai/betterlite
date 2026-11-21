mod calls;

use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::CatalogProviderInference;
use betterlite_config::sections::request_headers::Settings;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_headers::backend_call::{
    BackendCallArguments, backend_call_arguments,
};
use betterlite_core::api::request_headers::cache_control::{CacheDirective, parse_cache_control};
use betterlite_core::api::request_headers::cleaning::{clean_headers, logged_headers};
use betterlite_core::api::request_headers::header_metadata::add_header_metadata;
use betterlite_core::api::request_headers::provider_headers::provider_specific_headers;
use betterlite_core::api::request_headers::trace_context::{
    add_otel_trace_id, forward_traceparent,
};
use betterlite_core::api::request_headers::user_mapping::{
    internal_user_from_mapping, user_from_headers,
};
use betterlite_core::api::request_headers::{
    ForwardedHeaders, HeaderAuth, HeaderValue, RequestHeaders, SpanContext,
};
use betterlite_core::api::request_metadata_safety::metadata::MetadataField;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use calls::{
    BackendCall, CleanCall, Input, MetadataCall, ModelGroupCall, OtelCall, Pairs, TraceparentCall,
    UserCall,
};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!({
            "clean": input.clean.iter().map(clean_value).collect::<Vec<Value>>(),
            "backend": input.backend.iter().map(backend_value).collect::<Vec<Value>>(),
            "model_group": input.model_group.iter().map(model_group_value).collect::<Vec<Value>>(),
            "metadata": input.metadata.iter().map(metadata_value).collect::<Vec<Value>>(),
            "otel_trace_id": input.otel_trace_id.iter().map(otel_value).collect::<Vec<Value>>(),
            "traceparent": input.traceparent.iter().map(traceparent_value).collect::<Vec<Value>>(),
            "user": input.user.iter().map(user_value).collect::<Vec<Value>>(),
            "provider": input.provider.iter().map(provider_value).collect::<Vec<Value>>(),
            "cache_control": input.cache_control.iter().map(String::as_str).map(cache_value).collect::<Vec<Value>>(),
        }))
    }
}

fn configured<F>(change: F) -> Settings
where
    F: FnOnce(&mut Settings),
{
    let mut settings = Settings::default();
    change(&mut settings);
    settings
}

fn plain(headers: &Pairs) -> ForwardedHeaders {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), HeaderValue::Plain(value.clone())))
        .collect()
}

fn exposed(headers: &ForwardedHeaders) -> Value {
    let pairs: Map<String, Value> = headers
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.expose().to_owned())))
        .collect();
    Value::Object(pairs)
}

fn slot(name: &str) -> MetadataField {
    if name == "litellm_metadata" {
        MetadataField::LitellmMetadata
    } else {
        MetadataField::Metadata
    }
}

fn clean_value(call: &CleanCall) -> Value {
    let auth = HeaderAuth {
        litellm_key_header_name: call.litellm_key_header_name.clone(),
        forward_llm_provider_auth_headers: call.forward_auth_headers,
        authenticated_with_header: call.authenticated_with_header.clone(),
    };
    let headers: IndexMap<String, String> = call.headers.iter().cloned().collect();
    let cleaned = clean_headers(&headers, &auth);
    json!({ "forwarded": exposed(&cleaned), "logged": logged_headers(&cleaned) })
}

fn backend_value(call: &BackendCall) -> Value {
    let settings = configured(|chosen| {
        chosen.add_user_information_to_llm_headers = call.add_user_information;
        chosen.forward_openai_org_id = call.general_settings.forward_openai_org_id;
        chosen.forward_client_headers_to_llm_api =
            call.general_settings.forward_client_headers_to_llm_api;
    });
    match backend_call_arguments(
        &plain(&call.headers),
        &call.request_data,
        &call.principal,
        &settings,
    ) {
        Ok(arguments) => backend_json(&arguments),
        Err(_error) => json!({ "error": "invalid header" }),
    }
}

// LitellmDataForBackendLLMCall holds only the keys LiteLLM set
fn backend_json(arguments: &BackendCallArguments) -> Value {
    let fields = json!({
        "headers": arguments.headers.as_ref().map(exposed),
        "organization": arguments.organization,
        "timeout": arguments.timeout,
        "stream_timeout": arguments.stream_timeout,
        "client_side_timeout": arguments.client_side_timeout.then_some(true),
        "num_retries": arguments.num_retries,
        "keepalive_seconds": arguments.keepalive_seconds,
    });
    let Value::Object(set) = fields else {
        return fields;
    };
    Value::Object(
        set.into_iter()
            .filter(|(_name, value)| !value.is_null())
            .collect(),
    )
}

fn model_group_value(call: &ModelGroupCall) -> Value {
    let settings = configured(|chosen| {
        chosen.add_user_information_to_llm_headers = call.add_user_information;
        chosen.model_group_forwarding.clone_from(&call.models);
    });
    let inference = CatalogProviderInference::default();
    let aliases = BTreeMap::new();
    let model_access = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &inference,
        max_recurse_depth: 10,
    };
    let service = RequestHeaders {
        settings: &settings,
        model_access: &model_access,
        limits: ApiLimits::default(),
        otel_enabled: false,
    };
    let headers = service.model_group_headers(
        &call.data,
        &plain(&call.headers),
        &call.principal,
        &call.team_model_aliases,
    );
    match headers {
        Ok(Some(forwarded)) => json!({ "headers": exposed(&forwarded) }),
        Ok(None) => json!({ "headers": null }),
        Err(_error) => json!({ "error": "invalid header" }),
    }
}

fn metadata_value(call: &MetadataCall) -> Value {
    let mut data = call.data.clone();
    add_header_metadata(
        &plain(&call.headers),
        &mut data,
        slot(&call.slot),
        ApiLimits::default(),
    );
    json!(data)
}

fn otel_value(call: &OtelCall) -> Value {
    let mut data = call.data.clone();
    let span = call.span.map(|(trace_id, span_id)| SpanContext {
        trace_id: u128::from(trace_id),
        is_valid: trace_id != 0 && span_id != 0,
    });
    add_otel_trace_id(&mut data, slot(&call.slot), span);
    json!(data)
}

fn traceparent_value(call: &TraceparentCall) -> Value {
    let mut data = call.data.clone();
    let headers: IndexMap<String, String> = call.headers.iter().cloned().collect();
    forward_traceparent(&mut data, &headers, call.forward && call.otel_enabled);
    json!(data)
}

fn user_value(call: &UserCall) -> Value {
    let given = &call.general_settings;
    let settings = configured(|chosen| {
        chosen.user_header_mappings = given.user_header_mappings.clone().unwrap_or_default();
        chosen.user_header_name.clone_from(&given.user_header_name);
    });
    let headers = plain(&call.headers);
    let user_id = internal_user_from_mapping(&headers, &settings.user_header_mappings)
        .or_else(|| call.principal_user_id.clone());
    json!({ "user": user_from_headers(&headers, &settings), "user_id": user_id })
}

fn provider_value(headers: &Pairs) -> Value {
    let provider = provider_specific_headers(&plain(headers));
    let mut scoped: Vec<Value> = provider
        .scoped
        .iter()
        .map(|header| {
            json!({
                "custom_llm_provider": header.custom_llm_provider,
                "extra_headers": exposed(&header.extra_headers),
            })
        })
        .collect();
    // LiteLLM stores one scoped header as itself and several as a list
    let stored = match scoped.len() {
        0 => None,
        1 => scoped.pop(),
        _ => Some(Value::Array(scoped)),
    };
    let mut data = Map::new();
    if let Some(header) = stored {
        data.insert("provider_specific_header".to_owned(), header);
    }
    json!({ "data": data, "used_client_oauth_token": provider.used_client_oauth_token })
}

fn cache_value(cache_control: &str) -> Value {
    match parse_cache_control(cache_control) {
        Ok(directives) => {
            let parsed: Map<String, Value> = directives
                .into_iter()
                .map(|(key, directive)| {
                    let value = match directive {
                        CacheDirective::Flag => Value::Bool(true),
                        CacheDirective::Value(text) => Value::String(text),
                    };
                    (key, value)
                })
                .collect();
            Value::Object(parsed)
        }
        Err(_error) => json!({ "error": "invalid directive" }),
    }
}
